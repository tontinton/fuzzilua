const BUCKET_MAP: [u8; 256] = {
    let mut table = [0u8; 256];
    let mut i = 1usize;
    while i < 256 {
        table[i] = match i {
            1 => 1,
            2 => 2,
            3 => 4,
            4..=7 => 8,
            8..=15 => 16,
            16..=31 => 32,
            32..=127 => 64,
            _ => 128,
        };
        i += 1;
    }
    table
};

pub(crate) fn has_new_bits_slice(local: &[u8], global: &[u8]) -> bool {
    assert_eq!(
        local.len(),
        global.len(),
        "bitmap size mismatch in has_new_bits"
    );
    let chunks_l = local.chunks_exact(8);
    let chunks_g = global.chunks_exact(8);
    let rem_l = chunks_l.remainder();
    let rem_g = chunks_g.remainder();

    for (l, g) in chunks_l.zip(chunks_g) {
        let lw = u64::from_ne_bytes(l.try_into().unwrap());
        let gw = u64::from_ne_bytes(g.try_into().unwrap());
        if lw & !gw != 0 {
            return true;
        }
    }

    for (&l, &g) in rem_l.iter().zip(rem_g.iter()) {
        if l & !g != 0 {
            return true;
        }
    }

    false
}

pub(crate) fn merge_slice(src: &[u8], dst: &mut [u8]) {
    assert_eq!(src.len(), dst.len(), "bitmap size mismatch in merge");
    for (d, &s) in dst.iter_mut().zip(src.iter()) {
        *d |= s;
    }
}

pub(crate) fn count_nonzero(data: &[u8]) -> u32 {
    data.iter().filter(|&&b| b != 0).count() as u32
}

fn classify_slice(data: &mut [u8]) {
    for byte in data.iter_mut() {
        *byte = BUCKET_MAP[*byte as usize];
    }
}

pub struct CoverageBitmap {
    buf: Vec<u8>,
    edge_len: usize,
}

impl CoverageBitmap {
    pub fn new(edge_size: usize, gc_size: usize) -> Self {
        Self {
            buf: vec![0u8; edge_size + gc_size],
            edge_len: edge_size,
        }
    }

    pub fn from_raw(buf: Vec<u8>, edge_len: usize) -> Self {
        assert!(
            edge_len <= buf.len(),
            "edge_len ({edge_len}) exceeds buf length ({})",
            buf.len()
        );
        Self { buf, edge_len }
    }

    pub fn edge_bytes(&self) -> &[u8] {
        &self.buf[..self.edge_len]
    }

    pub fn gc_bytes(&self) -> &[u8] {
        &self.buf[self.edge_len..]
    }

    pub fn edge_bytes_mut(&mut self) -> &mut [u8] {
        &mut self.buf[..self.edge_len]
    }

    pub fn gc_bytes_mut(&mut self) -> &mut [u8] {
        &mut self.buf[self.edge_len..]
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.buf
    }

    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        &mut self.buf
    }

    pub fn edge_len(&self) -> usize {
        self.edge_len
    }

    pub fn gc_len(&self) -> usize {
        self.buf.len() - self.edge_len
    }

    pub fn has_new_bits(&self, global: &CoverageBitmap) -> bool {
        has_new_bits_slice(self.edge_bytes(), global.edge_bytes())
            || has_new_bits_slice(self.gc_bytes(), global.gc_bytes())
    }

    pub fn merge_into(&self, global: &mut CoverageBitmap) {
        merge_slice(self.edge_bytes(), global.edge_bytes_mut());
        merge_slice(self.gc_bytes(), global.gc_bytes_mut());
    }

    pub fn clear(&mut self) {
        self.buf.fill(0);
    }

    pub fn count_bits(&self) -> (u32, u32) {
        (
            count_nonzero(self.edge_bytes()),
            count_nonzero(self.gc_bytes()),
        )
    }

    pub fn classify_counts(&mut self) {
        classify_slice(&mut self.buf);
    }

    pub fn saturation(&self) -> f64 {
        if self.buf.is_empty() {
            return 0.0;
        }
        count_nonzero(&self.buf) as f64 / self.buf.len() as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_case::test_case;

    #[test]
    fn has_new_bits_detects_edge_gc_and_subset() {
        let global = CoverageBitmap::new(16, 16);

        let mut edge_only = CoverageBitmap::new(16, 16);
        edge_only.edge_bytes_mut()[5] = 1;
        assert!(edge_only.has_new_bits(&global));

        let mut gc_only = CoverageBitmap::new(16, 16);
        gc_only.gc_bytes_mut()[3] = 1;
        assert!(gc_only.has_new_bits(&global));

        let mut superset = CoverageBitmap::new(16, 16);
        superset.edge_bytes_mut()[2] = 0x0F;
        superset.gc_bytes_mut()[4] = 0x03;
        let mut global_wide = CoverageBitmap::new(16, 16);
        global_wide.edge_bytes_mut()[2] = 0xFF;
        global_wide.gc_bytes_mut()[4] = 0x0F;
        assert!(!superset.has_new_bits(&global_wide));
    }

    #[test]
    fn merge_into_idempotent() {
        let mut local = CoverageBitmap::new(16, 16);
        let mut global = CoverageBitmap::new(16, 16);
        local.edge_bytes_mut()[0] = 0xAA;
        local.gc_bytes_mut()[1] = 0x55;
        global.edge_bytes_mut()[0] = 0x11;

        local.merge_into(&mut global);
        let snapshot: Vec<u8> = global.as_bytes().to_vec();

        local.merge_into(&mut global);
        assert_eq!(global.as_bytes(), &snapshot[..]);
    }

    #[test]
    #[should_panic(expected = "bitmap size mismatch")]
    fn size_mismatch_panics() {
        let local = CoverageBitmap::new(16, 16);
        let global = CoverageBitmap::new(32, 32);
        local.has_new_bits(&global);
    }

    #[test_case(0, 0; "zero stays zero")]
    #[test_case(1, 1; "one maps to 1")]
    #[test_case(2, 2; "two maps to 2")]
    #[test_case(3, 4; "three maps to 4")]
    #[test_case(4, 8; "four maps to 8")]
    #[test_case(7, 8; "seven maps to 8")]
    #[test_case(8, 16; "eight maps to 16")]
    #[test_case(15, 16; "fifteen maps to 16")]
    #[test_case(16, 32; "sixteen maps to 32")]
    #[test_case(31, 32; "thirty_one maps to 32")]
    #[test_case(32, 64; "thirty_two maps to 64")]
    #[test_case(127, 64; "one_twenty_seven maps to 64")]
    #[test_case(128, 128; "one_twenty_eight maps to 128")]
    #[test_case(255, 128; "two_fifty_five maps to 128")]
    fn classify_counts_buckets(input: u8, expected: u8) {
        let mut bm = CoverageBitmap::new(1, 1);
        bm.edge_bytes_mut()[0] = input;
        bm.classify_counts();
        assert_eq!(bm.edge_bytes()[0], expected);
    }

    #[test]
    fn saturation_and_count_bits() {
        let empty = CoverageBitmap::new(10, 10);
        assert_eq!(empty.saturation(), 0.0);
        assert_eq!(empty.count_bits(), (0, 0));

        let mut partial = CoverageBitmap::new(10, 10);
        partial.edge_bytes_mut()[0] = 1;
        partial.edge_bytes_mut()[5] = 2;
        partial.gc_bytes_mut()[0] = 1;
        assert_eq!(partial.count_bits(), (2, 1));
        assert!((partial.saturation() - 3.0 / 20.0).abs() < 1e-10);
    }

    #[test]
    #[should_panic(expected = "edge_len")]
    fn from_raw_rejects_invalid_edge_len() {
        CoverageBitmap::from_raw(vec![0u8; 10], 11);
    }
}
