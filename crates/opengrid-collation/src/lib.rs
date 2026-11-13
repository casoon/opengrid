//! The collator of the browser (issue #391, rule S4a): a module of its own.
//!
//! The collation of a locale needs tables of some hundred kilobytes. The
//! engine module does not carry them; a page loads this module the first time
//! a query sorts by a locale, and gives the engine its one function
//! (`Engine.set_collator`). The engine then asks for the **sort keys** of a
//! column — bytes that compare as the locale orders the strings — and sorts by
//! those itself.
//!
//! The keys are the ones the engine makes on a server
//! (`opengrid_engine::collation::Icu`), so the two orders are one.

use opengrid_columns::sort::SortKeys;
use opengrid_engine::collation::{Collator, Icu};
use wasm_bindgen::prelude::*;

/// The sort keys of `locale` for a batch of strings.
///
/// `texts` is every string's UTF-8 one after the other and `ends` where each
/// one ends in it. The answer is packed the same way round: the end of every
/// key as a little-endian `u32`, then the keys' bytes — one buffer, since a
/// call across two modules copies each argument once.
#[wasm_bindgen]
pub fn sort_keys(locale: &str, texts: &[u8], ends: &[u32]) -> Result<Vec<u8>, JsError> {
    pack(locale, texts, ends).map_err(|message| JsError::new(&message))
}

fn pack(locale: &str, texts: &[u8], ends: &[u32]) -> Result<Vec<u8>, String> {
    let mut start = 0;
    let mut strings = Vec::with_capacity(ends.len());
    for end in ends {
        let end = *end as usize;
        let text = texts
            .get(start..end)
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .ok_or_else(|| "sort_keys: the texts are not what their ends say".to_owned())?;
        strings.push(text);
        start = end;
    }
    let mut keys = SortKeys::with_capacity(strings.len());
    Icu.sort_keys(locale, &mut strings.into_iter(), &mut keys)?;

    let mut packed = Vec::new();
    let mut end = 0u32;
    for row in 0..keys.len() {
        end = u32::try_from(keys.get(row).len())
            .ok()
            .and_then(|len| end.checked_add(len))
            .ok_or_else(|| "sort_keys: the keys are more than four gigabytes".to_owned())?;
        packed.extend_from_slice(&end.to_le_bytes());
    }
    for row in 0..keys.len() {
        packed.extend_from_slice(keys.get(row));
    }
    Ok(packed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The keys order as the locale does: `ä` with `a` in German, after `z`
    /// in Swedish.
    #[test]
    fn the_packed_keys_order_as_the_locale() {
        let words = ["zebra", "ähnlich", "apfel"];
        let texts = words.concat().into_bytes();
        let mut ends = Vec::new();
        let mut end = 0;
        for word in words {
            end += word.len() as u32;
            ends.push(end);
        }
        let order = |locale: &str| {
            let packed = pack(locale, &texts, &ends).unwrap();
            let (head, bytes) = packed.split_at(4 * words.len());
            let mut keys = Vec::new();
            let mut start = 0;
            for end in head.chunks(4) {
                let end = u32::from_le_bytes(end.try_into().unwrap()) as usize;
                keys.push(&bytes[start..end]);
                start = end;
            }
            let mut order: Vec<usize> = (0..words.len()).collect();
            order.sort_by_key(|at| keys[*at]);
            order.into_iter().map(|at| words[at]).collect::<Vec<_>>()
        };
        assert_eq!(order("de"), ["ähnlich", "apfel", "zebra"]);
        assert_eq!(order("sv"), ["apfel", "zebra", "ähnlich"]);
    }

    #[test]
    fn ends_that_do_not_fit_the_texts_are_refused() {
        assert!(pack("de", b"ab", &[5]).is_err());
        assert!(pack("de", &[0xFF], &[1]).is_err());
    }
}
