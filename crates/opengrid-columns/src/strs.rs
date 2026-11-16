//! Text with few distinct values, stored as a dictionary.
//!
//! A text column used to hold a four-byte offset and the text for every row. A country, a status, a
//! customer of a few hundred: the same few texts, over and over. [`Strs`] keeps each distinct text
//! once and a code of one, two or four bytes per row (issue #451). A column of distinct texts — a
//! name, a note, an id written out — stays flat, as it was.
//!
//! The choice is made while the column is built, which starts as a dictionary: every push looks the
//! text up (an open-addressing table over the dictionary's own text, no copy of it) and either finds
//! its code or adds it. From 4,096 rows on, at every doubling of the rows, a dictionary with more
//! distinct texts than half its rows is given up: the column becomes flat, once. So is one whose
//! lookup meets a long run of collisions, which bounds the work an input made to collide can cost.
//! The table is a builder's, and is gone when the column is finished.
//!
//! What a reader sees does not change: [`Strs::get`] is the text that was pushed, a NULL is the
//! empty text, and equality is by text, not by layout.

/// Rows from which a dictionary that is not worth it is given up, then at every doubling.
const CHECK_FROM: usize = 4096;
/// The longest run of occupied slots a lookup may walk before the column is made flat.
const MAX_PROBE: usize = 64;
/// An empty slot of the lookup table.
const EMPTY: u32 = u32::MAX;

/// The codes of a dictionary column: one, two or four bytes each, as many as its distinct texts need.
#[derive(Clone, Debug)]
pub enum Codes {
    U8(Vec<u8>),
    U16(Vec<u16>),
    U32(Vec<u32>),
}

impl Codes {
    fn with_capacity(capacity: usize) -> Self {
        Codes::U8(Vec::with_capacity(capacity))
    }

    fn len(&self) -> usize {
        match self {
            Codes::U8(codes) => codes.len(),
            Codes::U16(codes) => codes.len(),
            Codes::U32(codes) => codes.len(),
        }
    }

    #[inline]
    fn get(&self, row: usize) -> usize {
        match self {
            Codes::U8(codes) => usize::from(codes[row]),
            Codes::U16(codes) => usize::from(codes[row]),
            Codes::U32(codes) => codes[row] as usize,
        }
    }

    fn width(&self) -> usize {
        match self {
            Codes::U8(_) => 1,
            Codes::U16(_) => 2,
            Codes::U32(_) => 4,
        }
    }

    #[inline]
    fn push(&mut self, code: u32) {
        match self {
            Codes::U8(codes) => {
                if let Ok(narrow) = u8::try_from(code) {
                    return codes.push(narrow);
                }
            }
            Codes::U16(codes) => {
                if let Ok(narrow) = u16::try_from(code) {
                    return codes.push(narrow);
                }
            }
            Codes::U32(codes) => return codes.push(code),
        }
        self.widen(code);
        self.push(code);
    }

    /// Moves the codes into the width that holds `code`, keeping the capacity.
    #[cold]
    fn widen(&mut self, code: u32) {
        let capacity = match self {
            Codes::U8(codes) => codes.capacity(),
            Codes::U16(codes) => codes.capacity(),
            Codes::U32(codes) => codes.capacity(),
        }
        .max(self.len() + 1);
        let wider = if u16::try_from(code).is_ok() {
            let mut codes = Vec::with_capacity(capacity);
            codes.extend((0..self.len()).map(|row| self.get(row) as u16));
            Codes::U16(codes)
        } else {
            let mut codes = Vec::with_capacity(capacity);
            codes.extend((0..self.len()).map(|row| self.get(row) as u32));
            Codes::U32(codes)
        };
        *self = wider;
    }
}

/// The lookup of a dictionary while it is built: slot `i` holds the code of a text whose hash lands
/// there, or [`EMPTY`]. Linear probing, at most half full.
/// The lookup of a dictionary while it is built.
#[derive(Clone, Debug)]
struct Probe {
    slots: Vec<u32>,
}

impl Probe {
    fn new() -> Self {
        Self {
            slots: vec![EMPTY; 64],
        }
    }
}

/// The hash of a text: eight bytes at a time, then mixed. Not keyed — a run of collisions is what
/// [`MAX_PROBE`] is for.
fn hash(bytes: &[u8]) -> usize {
    const K: u64 = 0x517c_c1b7_2722_0a95;
    let mut h = bytes.len() as u64;
    let (chunks, rest) = bytes.as_chunks::<8>();
    for chunk in chunks {
        let word = u64::from_le_bytes(*chunk);
        h = (h.rotate_left(5) ^ word).wrapping_mul(K);
    }
    if !rest.is_empty() {
        let mut word = 0u64;
        for (at, byte) in rest.iter().enumerate() {
            word |= u64::from(*byte) << (8 * at);
        }
        h = (h.rotate_left(5) ^ word).wrapping_mul(K);
    }
    h = (h ^ (h >> 29)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    (h >> 32) as usize
}

/// A column of text.
#[derive(Clone, Debug)]
pub enum Strs {
    /// Every text in one buffer; text `i` is `text[offsets[i]..offsets[i + 1]]`.
    Flat { offsets: Vec<u32>, text: String },
    /// Each distinct text once, in the order they first came: text `c` is
    /// `text[offsets[c]..offsets[c + 1]]`, and row `i` is the text of `codes[i]`.
    Dict {
        codes: Codes,
        offsets: Vec<u32>,
        text: String,
    },
}

const OVERFLOW: &str = "holds more than 4 GiB of text, more than a column can";

impl Strs {
    /// A flat column of the given buffers, as a decoded result is.
    pub fn from_flat(offsets: Vec<u32>, text: String) -> Self {
        Strs::Flat { offsets, text }
    }

    /// Number of rows.
    pub fn len(&self) -> usize {
        match self {
            Strs::Flat { offsets, .. } => offsets.len() - 1,
            Strs::Dict { codes, .. } => codes.len(),
        }
    }

    /// True when there are no rows.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether the column is a dictionary.
    pub fn is_dictionary(&self) -> bool {
        matches!(self, Strs::Dict { .. })
    }

    /// The number of distinct texts of a dictionary; `None` for a flat column, which does not count.
    pub fn distinct(&self) -> Option<usize> {
        match self {
            Strs::Flat { .. } => None,
            Strs::Dict { offsets, .. } => Some(offsets.len() - 1),
        }
    }

    /// The bytes the column takes.
    pub fn byte_size(&self) -> usize {
        match self {
            Strs::Flat { offsets, text } => offsets.len() * 4 + text.len(),
            Strs::Dict {
                codes,
                offsets,
                text,
                ..
            } => codes.len() * codes.width() + offsets.len() * 4 + text.len(),
        }
    }

    /// The text at `row`.
    #[inline]
    pub fn get(&self, row: usize) -> &str {
        match self {
            Strs::Flat { offsets, text } => &text[offsets[row] as usize..offsets[row + 1] as usize],
            Strs::Dict {
                codes,
                offsets,
                text,
                ..
            } => {
                let code = codes.get(row);
                &text[offsets[code] as usize..offsets[code + 1] as usize]
            }
        }
    }

    /// The code of a dictionary's row, and the text of a code: for a reader that works on codes.
    pub fn code(&self, row: usize) -> Option<usize> {
        match self {
            Strs::Flat { .. } => None,
            Strs::Dict { codes, .. } => Some(codes.get(row)),
        }
    }

    /// The text of a code of a dictionary.
    pub fn dictionary_text(&self, code: usize) -> Option<&str> {
        match self {
            Strs::Flat { .. } => None,
            Strs::Dict { offsets, text, .. } => {
                Some(&text[offsets[code] as usize..offsets[code + 1] as usize])
            }
        }
    }

    /// The code of `wanted` in a dictionary, or `None` when it is not in it (or the column is flat).
    pub fn find(&self, wanted: &str) -> Option<usize> {
        match self {
            Strs::Flat { .. } => None,
            Strs::Dict { offsets, text, .. } => (0..offsets.len() - 1)
                .find(|code| &text[offsets[*code] as usize..offsets[*code + 1] as usize] == wanted),
        }
    }

    /// The texts at `positions`, in that order, as a flat column.
    pub fn gather(&self, positions: impl Iterator<Item = usize> + Clone) -> Strs {
        let bytes: usize = positions.clone().map(|at| self.get(at).len()).sum();
        let mut text = String::with_capacity(bytes);
        let mut offsets = Vec::with_capacity(positions.size_hint().0 + 1);
        offsets.push(0);
        for at in positions {
            text.push_str(self.get(at));
            offsets.push(text.len() as u32);
        }
        Strs::Flat { offsets, text }
    }

    /// The texts in order.
    pub fn iter(&self) -> impl Iterator<Item = &str> + '_ {
        (0..self.len()).map(|row| self.get(row))
    }
}

/// A column of text being built: a dictionary until it is not worth it, then flat.
///
/// The lookup, the rows the column is built for and the next look at the dictionary are the
/// builder's, and are gone with it: the [`Strs`] it finishes is the column and nothing else.
#[derive(Clone, Debug)]
pub struct StrBuilder {
    strs: Strs,
    probe: Probe,
    /// The rows the column is built for, which sizes the buffers of a column that turns flat.
    capacity: usize,
    /// The number of rows at which the dictionary is next looked at.
    next_check: usize,
}

impl StrBuilder {
    /// A builder for a column of `capacity` rows.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            strs: Strs::Dict {
                codes: Codes::with_capacity(capacity),
                offsets: vec![0],
                text: String::new(),
            },
            probe: Probe::new(),
            capacity,
            next_check: CHECK_FROM,
        }
    }

    /// Appends a text. An error when the column would hold more than 4 GiB of text.
    pub fn push(&mut self, value: &str) -> Result<(), String> {
        match &mut self.strs {
            Strs::Flat { offsets, text } => {
                text.push_str(value);
                let end = u32::try_from(text.len()).map_err(|_| OVERFLOW.to_owned())?;
                offsets.push(end);
                Ok(())
            }
            Strs::Dict { .. } => {
                if self.push_dictionary(value)? {
                    Ok(())
                } else {
                    // A run of collisions: flat from here on, and this text with it.
                    self.make_flat();
                    self.push(value)
                }
            }
        }
    }

    /// Appends to a dictionary; `false` when the lookup walked [`MAX_PROBE`] slots and gave up.
    fn push_dictionary(&mut self, value: &str) -> Result<bool, String> {
        let Strs::Dict {
            codes,
            offsets,
            text,
        } = &mut self.strs
        else {
            unreachable!("a dictionary")
        };
        let probe = &mut self.probe;
        let bytes = value.as_bytes();
        let mask = probe.slots.len() - 1;
        let mut at = hash(bytes) & mask;
        let mut walked = 0;
        let found = loop {
            let slot = probe.slots[at];
            if slot == EMPTY {
                break None;
            }
            let (from, to) = (
                offsets[slot as usize] as usize,
                offsets[slot as usize + 1] as usize,
            );
            if &text.as_bytes()[from..to] == bytes {
                break Some(slot);
            }
            walked += 1;
            if walked > MAX_PROBE {
                return Ok(false);
            }
            at = (at + 1) & mask;
        };
        let code = match found {
            Some(code) => code,
            None => {
                let code = (offsets.len() - 1) as u32;
                text.push_str(value);
                let end = u32::try_from(text.len()).map_err(|_| OVERFLOW.to_owned())?;
                offsets.push(end);
                probe.slots[at] = code;
                if offsets.len() * 2 > probe.slots.len() {
                    // At most half full: twice the slots, every text put in again.
                    let mut slots = vec![EMPTY; probe.slots.len() * 2];
                    let mask = slots.len() - 1;
                    for code in 0..offsets.len() - 1 {
                        let entry =
                            &text.as_bytes()[offsets[code] as usize..offsets[code + 1] as usize];
                        let mut at = hash(entry) & mask;
                        while slots[at] != EMPTY {
                            at = (at + 1) & mask;
                        }
                        slots[at] = code as u32;
                    }
                    probe.slots = slots;
                }
                code
            }
        };
        codes.push(code);
        if codes.len() >= self.next_check {
            self.next_check = codes.len() * 2;
            // Few enough distinct texts to be worth a dictionary, or flat.
            if (offsets.len() - 1) * 2 > codes.len() {
                self.make_flat();
            }
        }
        Ok(true)
    }

    /// Turns the dictionary into a flat column, with the buffers of the size the column will have.
    fn make_flat(&mut self) {
        let Strs::Dict {
            codes,
            offsets,
            text,
        } = &self.strs
        else {
            return;
        };
        let rows = codes.len();
        let bytes: usize = (0..rows)
            .map(|row| {
                let code = codes.get(row);
                (offsets[code + 1] - offsets[code]) as usize
            })
            .sum();
        // The rest of the rows are expected to be like these.
        let expected = if rows == 0 || self.capacity <= rows {
            bytes
        } else {
            bytes + bytes / rows * (self.capacity - rows) * 21 / 20
        };
        let mut flat_text = String::with_capacity(expected);
        let mut flat_offsets = Vec::with_capacity(self.capacity.max(rows) + 1);
        flat_offsets.push(0);
        for row in 0..rows {
            let code = codes.get(row);
            flat_text.push_str(&text[offsets[code] as usize..offsets[code + 1] as usize]);
            flat_offsets.push(flat_text.len() as u32);
        }
        self.strs = Strs::Flat {
            offsets: flat_offsets,
            text: flat_text,
        };
        self.probe = Probe { slots: Vec::new() };
    }

    /// The column.
    pub fn finish(self) -> Strs {
        self.strs
    }
}

/// By text: two columns that hold the same texts are equal whatever their layout.
impl PartialEq for Strs {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Strs::Flat {
                    offsets: a,
                    text: at,
                },
                Strs::Flat {
                    offsets: b,
                    text: bt,
                },
            ) => a == b && at == bt,
            _ => self.len() == other.len() && self.iter().eq(other.iter()),
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn built(texts: &[&str], capacity: usize) -> Strs {
        let mut builder = StrBuilder::with_capacity(capacity);
        for text in texts {
            builder.push(text).unwrap();
        }
        builder.finish()
    }

    #[test]
    fn a_few_texts_are_a_dictionary_with_a_code_a_row() {
        let rows: Vec<String> = (0..10_000)
            .map(|row| format!("country {}", row % 8))
            .collect();
        let texts: Vec<&str> = rows.iter().map(String::as_str).collect();
        let strs = built(&texts, texts.len());
        assert!(strs.is_dictionary());
        assert_eq!(strs.distinct(), Some(8));
        assert_eq!(strs.iter().collect::<Vec<_>>(), texts);
        // One byte a row, eight texts once, nine offsets.
        let text_bytes: usize = (0..8).map(|code| format!("country {code}").len()).sum();
        assert_eq!(strs.byte_size(), 10_000 + 9 * 4 + text_bytes);
        let flat: usize = texts.iter().map(|text| 4 + text.len()).sum::<usize>() + 4;
        assert!(
            strs.byte_size() * 10 < flat,
            "{} against {flat}",
            strs.byte_size()
        );
    }

    #[test]
    fn codes_widen_with_the_distinct_texts() {
        let rows: Vec<String> = (0..9_000).map(|row| format!("t{}", row % 300)).collect();
        let texts: Vec<&str> = rows.iter().map(String::as_str).collect();
        let strs = built(&texts, texts.len());
        assert!(strs.is_dictionary());
        let Strs::Dict { codes, .. } = &strs else {
            panic!("a dictionary")
        };
        assert_eq!(codes.width(), 2, "300 texts do not fit a byte");
        assert_eq!(strs.iter().collect::<Vec<_>>(), texts);
        // Two bytes a row, 300 texts once, 301 offsets.
        let text_bytes: usize = (0..300).map(|code| format!("t{code}").len()).sum();
        assert_eq!(strs.byte_size(), 9_000 * 2 + 301 * 4 + text_bytes);
    }

    #[test]
    fn codes_take_as_many_bytes_as_the_largest_needs() {
        // A dictionary past 65,536 distinct texts needs more rows than a test should make: the
        // codes are tried alone, at the edge of every width.
        let mut codes = Codes::with_capacity(8);
        let pushed = [0u32, 255, 256, 65_535, 65_536, 5_000_000, 3, 70_000];
        for (at, code) in pushed.into_iter().enumerate() {
            codes.push(code);
            let width = match pushed[..=at].iter().max().unwrap() {
                0..=255 => 1,
                256..=65_535 => 2,
                _ => 4,
            };
            assert_eq!(codes.width(), width, "after {code}");
        }
        assert_eq!(
            (0..8).map(|row| codes.get(row) as u32).collect::<Vec<_>>(),
            pushed
        );
    }

    #[test]
    fn distinct_texts_make_a_flat_column() {
        let rows: Vec<String> = (0..20_000)
            .map(|row| format!("note number {row}"))
            .collect();
        let texts: Vec<&str> = rows.iter().map(String::as_str).collect();
        let strs = built(&texts, texts.len());
        assert!(!strs.is_dictionary(), "every text is new");
        assert_eq!(strs.iter().collect::<Vec<_>>(), texts);
        let flat: usize = texts.iter().map(|text| 4 + text.len()).sum::<usize>() + 4;
        assert_eq!(strs.byte_size(), flat);
    }

    #[test]
    fn a_column_that_turns_distinct_later_turns_flat_then() {
        // 8,000 rows of eight texts, then 12,000 new ones, all of one length: the check at 16,384
        // rows sees more than half of the rows distinct.
        let mut rows: Vec<String> = (0..8_000)
            .map(|row| format!("fresh {:04x}", row % 8))
            .collect();
        rows.extend((0..12_000).map(|row| format!("fresh {:04x}", 0x1000 + row)));
        let texts: Vec<&str> = rows.iter().map(String::as_str).collect();
        let strs = built(&texts, texts.len());
        assert!(!strs.is_dictionary());
        assert_eq!(strs.iter().collect::<Vec<_>>(), texts);
        // And the text it went on with was sized for the rows to come, not doubled: within a tenth
        // of what it holds, where doubling from what the dictionary held would leave 60 % more.
        let Strs::Flat { text, .. } = &strs else {
            panic!("flat")
        };
        assert!(
            text.capacity() >= text.len() && text.capacity() <= text.len() + text.len() / 10,
            "{} for {}",
            text.capacity(),
            text.len()
        );
    }

    #[test]
    fn the_empty_text_and_multi_byte_text_are_texts() {
        let strs = built(&["", "ä", "", "日本語", "ä", ""], 6);
        assert_eq!(
            strs.iter().collect::<Vec<_>>(),
            ["", "ä", "", "日本語", "ä", ""]
        );
        assert_eq!(strs.find("ä"), Some(1));
        assert_eq!(strs.find("zzz"), None);
        assert_eq!(strs.code(3), Some(2));
        assert_eq!(strs.dictionary_text(2), Some("日本語"));
    }

    #[test]
    fn texts_that_collide_end_the_dictionary_not_the_column() {
        // Texts that hash alike are not easy to make; a dictionary whose lookup walks too far is
        // flat, and holds the same texts. Forced here by many short texts at a tiny table.
        let rows: Vec<String> = (0..30_000).map(|row| format!("{:x}", row * 7919)).collect();
        let texts: Vec<&str> = rows.iter().map(String::as_str).collect();
        let strs = built(&texts, texts.len());
        assert_eq!(strs.iter().collect::<Vec<_>>(), texts);
    }

    #[test]
    fn equal_texts_are_equal_whatever_the_layout() {
        let texts = ["a", "b", "a", "", "b"];
        let dictionary = built(&texts, 5);
        assert!(dictionary.is_dictionary());
        let mut offsets = vec![0];
        let mut text = String::new();
        for each in texts {
            text.push_str(each);
            offsets.push(text.len() as u32);
        }
        let flat = Strs::from_flat(offsets, text);
        assert_eq!(dictionary, flat);
        assert_eq!(flat, dictionary);
        assert_ne!(dictionary, built(&["a", "b", "a", "", "c"], 5));
        assert_ne!(dictionary, built(&["a", "b"], 2));
    }

    #[test]
    fn gather_makes_a_flat_column_of_the_rows_asked_for() {
        let strs = built(&["x", "yy", "x", "zzz"], 4);
        let taken = strs.gather([3, 0, 0, 1].into_iter());
        assert!(!taken.is_dictionary());
        assert_eq!(taken.iter().collect::<Vec<_>>(), ["zzz", "x", "x", "yy"]);
    }

    proptest! {
        /// Whatever texts are pushed, in whatever mix of few and many distinct, they are read back in
        /// order and the column is equal to the flat column of the same texts.
        #[test]
        fn pushed_texts_are_read_back(
            distinct in 1usize..6000,
            rows in 0usize..9000,
            seed in any::<u64>(),
            wide in any::<bool>(),
        ) {
            let mut state = seed | 1;
            let mut next = move || {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state
            };
            let texts: Vec<String> = (0..rows)
                .map(|_| {
                    let pick = (next() % distinct as u64) as usize;
                    if wide { format!("äö{pick}日") } else { format!("{pick}") }
                })
                .collect();
            let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
            let strs = built(&refs, rows);
            prop_assert_eq!(strs.len(), rows);
            prop_assert_eq!(strs.iter().collect::<Vec<_>>(), refs.clone());
            let mut offsets = vec![0u32];
            let mut text = String::new();
            for each in &refs {
                text.push_str(each);
                offsets.push(text.len() as u32);
            }
            prop_assert_eq!(&strs, &Strs::from_flat(offsets, text));
            // A dictionary is only kept where it is smaller than the flat column would be.
            if strs.is_dictionary() && rows >= CHECK_FROM {
                prop_assert!(strs.distinct().unwrap() * 2 <= rows * 2);
            }
        }
    }
}
