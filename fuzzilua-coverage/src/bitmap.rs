use serde::de::{self, SeqAccess, Visitor};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// AFL-style hit count bucketization: maps raw byte counts to power-of-2 buckets.
/// Applied after each execution to normalize coverage before comparison.
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

fn is_subset_slice(sub: &[u8], sup: &[u8]) -> bool {
    assert_eq!(sub.len(), sup.len(), "bitmap size mismatch in is_subset");
    sub.iter().zip(sup.iter()).all(|(&s, &g)| s & !g == 0)
}

#[derive(Clone, Debug)]
pub struct CoverageBitmap {
    buf: Vec<u8>,
    edge_len: usize,
}

impl Serialize for CoverageBitmap {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let sparse: Vec<(u32, u8)> = self
            .buf
            .iter()
            .enumerate()
            .filter(|&(_, v)| *v != 0)
            .map(|(i, &v)| (i as u32, v))
            .collect();
        let mut s = serializer.serialize_struct("CoverageBitmap", 3)?;
        s.serialize_field("edge_len", &self.edge_len)?;
        s.serialize_field("total_len", &self.buf.len())?;
        s.serialize_field("sparse", &sparse)?;
        s.end()
    }
}

impl<'de> Deserialize<'de> for CoverageBitmap {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct BitmapVisitor;

        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "snake_case")]
        enum Field {
            EdgeLen,
            TotalLen,
            Sparse,
            #[serde(other)]
            Buf,
        }

        impl<'de> Visitor<'de> for BitmapVisitor {
            type Value = CoverageBitmap;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("CoverageBitmap")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let edge_len: usize = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;
                let total_len: usize = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                if total_len > 16 * 1024 * 1024 {
                    return Err(de::Error::custom("total_len too large"));
                }
                if edge_len > total_len {
                    return Err(de::Error::custom("edge_len > total_len"));
                }
                let sparse: Vec<(u32, u8)> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                let mut buf = vec![0u8; total_len];
                for (idx, val) in sparse {
                    if (idx as usize) < total_len {
                        buf[idx as usize] = val;
                    }
                }
                Ok(CoverageBitmap { buf, edge_len })
            }

            fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut edge_len: Option<usize> = None;
                let mut total_len: Option<usize> = None;
                let mut sparse: Option<Vec<(u32, u8)>> = None;
                let mut legacy_buf: Option<Vec<u8>> = None;

                while let Some(key) = map.next_key()? {
                    match key {
                        Field::EdgeLen => edge_len = Some(map.next_value()?),
                        Field::TotalLen => total_len = Some(map.next_value()?),
                        Field::Sparse => sparse = Some(map.next_value()?),
                        Field::Buf => legacy_buf = Some(map.next_value()?),
                    }
                }

                let edge_len = edge_len.ok_or_else(|| de::Error::missing_field("edge_len"))?;

                if let Some(sparse) = sparse {
                    let total_len =
                        total_len.ok_or_else(|| de::Error::missing_field("total_len"))?;
                    let mut buf = vec![0u8; total_len];
                    for (idx, val) in sparse {
                        if (idx as usize) < total_len {
                            buf[idx as usize] = val;
                        }
                    }
                    Ok(CoverageBitmap { buf, edge_len })
                } else if let Some(buf) = legacy_buf {
                    Ok(CoverageBitmap { buf, edge_len })
                } else {
                    Err(de::Error::missing_field("sparse"))
                }
            }
        }

        deserializer.deserialize_struct(
            "CoverageBitmap",
            &["edge_len", "total_len", "sparse"],
            BitmapVisitor,
        )
    }
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

    pub fn is_subset_of(&self, other: &CoverageBitmap) -> bool {
        is_subset_slice(self.edge_bytes(), other.edge_bytes())
            && is_subset_slice(self.gc_bytes(), other.gc_bytes())
    }

    pub fn is_empty(&self) -> bool {
        self.buf.iter().all(|&b| b == 0)
    }

    pub fn total_nonzero(&self) -> u32 {
        count_nonzero(&self.buf)
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
