use crate::types::Variable;

/// Bitset indexed by `Variable.0`. O(1) insert/contains, used by validate,
/// lift, and mutators instead of `HashSet<Variable>`.
#[derive(Clone, Debug, PartialEq)]
pub struct VarBitset {
    bits: Vec<u64>,
}

const BITS_PER_WORD: usize = 64;

impl VarBitset {
    pub fn new() -> Self {
        Self { bits: Vec::new() }
    }

    const MAX_REASONABLE_VARS: u32 = 1_000_000;

    pub fn with_capacity(max_var_id: u32) -> Self {
        let capped = max_var_id.min(Self::MAX_REASONABLE_VARS);
        let words = (capped as usize / BITS_PER_WORD) + 1;
        Self {
            bits: vec![0; words],
        }
    }

    fn ensure_capacity(&mut self, var_id: u32) {
        let word = var_id as usize / BITS_PER_WORD;
        if word >= self.bits.len() {
            self.bits.resize(word + 1, 0);
        }
    }

    pub fn insert(&mut self, v: Variable) -> bool {
        self.ensure_capacity(v.0);
        let word = v.0 as usize / BITS_PER_WORD;
        let bit = 1u64 << (v.0 as usize % BITS_PER_WORD);
        let was_set = self.bits[word] & bit != 0;
        self.bits[word] |= bit;
        !was_set
    }

    pub fn contains(&self, v: &Variable) -> bool {
        let word = v.0 as usize / BITS_PER_WORD;
        if word >= self.bits.len() {
            return false;
        }
        self.bits[word] & (1u64 << (v.0 as usize % BITS_PER_WORD)) != 0
    }
}

impl Default for VarBitset {
    fn default() -> Self {
        Self::new()
    }
}
