use std::ffi::CString;

use memmap2::MmapMut;
use nix::fcntl::OFlag;
use nix::sys::mman::{shm_open, shm_unlink};
use nix::sys::stat::Mode;
use nix::unistd::ftruncate;

use crate::bitmap::CoverageBitmap;

pub struct SharedCoverage {
    name: CString,
    mmap: MmapMut,
    edge_len: usize,
}

impl SharedCoverage {
    pub fn create(name: &str, edge_size: usize, gc_size: usize) -> std::io::Result<Self> {
        let total = edge_size + gc_size;
        let cname = CString::new(name)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

        let fd = shm_open(
            cname.as_c_str(),
            OFlag::O_CREAT | OFlag::O_RDWR | OFlag::O_EXCL,
            Mode::S_IRUSR | Mode::S_IWUSR,
        )
        .map_err(std::io::Error::other)?;

        ftruncate(&fd, total as nix::libc::off_t).map_err(std::io::Error::other)?;

        let mmap = unsafe { MmapMut::map_mut(&fd)? };

        Ok(Self {
            name: cname,
            mmap,
            edge_len: edge_size,
        })
    }

    pub fn open(name: &str, edge_size: usize, gc_size: usize) -> std::io::Result<Self> {
        let expected_total = edge_size + gc_size;
        let cname = CString::new(name)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

        let fd = shm_open(cname.as_c_str(), OFlag::O_RDWR, Mode::empty())
            .map_err(std::io::Error::other)?;

        let mmap = unsafe { MmapMut::map_mut(&fd)? };

        if mmap.len() != expected_total {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "shm size mismatch: expected {expected_total}, got {}",
                    mmap.len()
                ),
            ));
        }

        Ok(Self {
            name: cname,
            mmap,
            edge_len: edge_size,
        })
    }

    pub fn read_into(&self, dst: &mut CoverageBitmap) {
        assert_eq!(
            self.mmap.len(),
            dst.as_bytes().len(),
            "SharedCoverage size ({}) != CoverageBitmap size ({})",
            self.mmap.len(),
            dst.as_bytes().len(),
        );
        assert_eq!(
            self.edge_len,
            dst.edge_len(),
            "SharedCoverage edge_len ({}) != CoverageBitmap edge_len ({})",
            self.edge_len,
            dst.edge_len(),
        );
        dst.as_bytes_mut().copy_from_slice(&self.mmap);
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.mmap
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.mmap
    }

    pub fn clear(&mut self) {
        self.mmap.fill(0);
    }
}

impl Drop for SharedCoverage {
    fn drop(&mut self) {
        let _ = shm_unlink(self.name.as_c_str());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_name(test: &str) -> String {
        format!("/fuzzilua_test_{test}_{}", std::process::id())
    }

    #[test]
    fn create_write_open_read() {
        let name = unique_name("create_write_open_read");
        let pattern: Vec<u8> = (0u8..128).cycle().take(256).collect();

        {
            let mut shm = SharedCoverage::create(&name, 128, 128).unwrap();
            shm.as_mut_slice().copy_from_slice(&pattern);

            let shm2 = SharedCoverage::open(&name, 128, 128).unwrap();
            assert_eq!(shm2.as_slice(), &pattern[..]);
        }
    }

    #[test]
    fn read_into_copies_to_bitmap() {
        let name = unique_name("read_into_copies");
        let mut shm = SharedCoverage::create(&name, 16, 16).unwrap();
        shm.as_mut_slice()[0] = 0xAA;
        shm.as_mut_slice()[16] = 0xBB;

        let mut bm = CoverageBitmap::new(16, 16);
        shm.read_into(&mut bm);

        assert_eq!(bm.edge_bytes()[0], 0xAA);
        assert_eq!(bm.gc_bytes()[0], 0xBB);
    }

    #[test]
    fn open_with_wrong_size_fails() {
        let name = unique_name("wrong_size");
        let _shm = SharedCoverage::create(&name, 64, 64).unwrap();
        let result = SharedCoverage::open(&name, 128, 128);
        match result {
            Err(e) => assert_eq!(e.kind(), std::io::ErrorKind::InvalidData),
            Ok(_) => panic!("should fail on size mismatch"),
        }
    }

    #[test]
    fn drop_cleans_up() {
        let name = unique_name("drop_cleans_up");
        {
            let _shm = SharedCoverage::create(&name, 64, 64).unwrap();
        }
        let result = SharedCoverage::create(&name, 64, 64);
        assert!(result.is_ok());
    }
}
