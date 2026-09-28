//! A packed bitmap: which cells of a column hold a value.

/// One bit per row, set when the row holds a value, cleared when it is NULL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bitmap {
    words: Vec<u64>,
    len: usize,
}

impl Bitmap {
    /// An empty bitmap with room for `capacity` bits.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            words: Vec::with_capacity(capacity.div_ceil(64)),
            len: 0,
        }
    }

    /// `len` bits, all set.
    pub fn all_set(len: usize) -> Self {
        let mut bitmap = Self::with_capacity(len);
        for _ in 0..len {
            bitmap.push(true);
        }
        bitmap
    }

    /// Appends one bit.
    pub fn push(&mut self, set: bool) {
        let bit = self.len % 64;
        if bit == 0 {
            self.words.push(0);
        }
        if set {
            *self.words.last_mut().expect("a word was pushed") |= 1 << bit;
        }
        self.len += 1;
    }

    /// The bit at `index`.
    pub fn get(&self, index: usize) -> bool {
        debug_assert!(index < self.len, "bit {index} of {}", self.len);
        self.words[index / 64] & (1 << (index % 64)) != 0
    }

    /// Number of bits.
    pub fn len(&self) -> usize {
        self.len
    }

    /// True when the bitmap holds no bits.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Number of cleared bits — the NULLs of a validity bitmap.
    pub fn count_cleared(&self) -> usize {
        let set: usize = self
            .words
            .iter()
            .map(|word| word.count_ones() as usize)
            .sum();
        self.len - set
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_come_back_as_pushed_across_word_boundaries() {
        let pattern: Vec<bool> = (0..200).map(|i| i % 3 == 0 || i == 64).collect();
        let mut bitmap = Bitmap::with_capacity(0);
        for bit in &pattern {
            bitmap.push(*bit);
        }
        assert_eq!(bitmap.len(), 200);
        for (index, bit) in pattern.iter().enumerate() {
            assert_eq!(bitmap.get(index), *bit, "bit {index}");
        }
        let cleared = pattern.iter().filter(|bit| !**bit).count();
        assert_eq!(bitmap.count_cleared(), cleared);
    }
}
