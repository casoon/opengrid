//! The collation of a locale for a sort key (rule S4a, issue #391).
//!
//! The engine does not order text by a locale itself. It asks a [`Collator`]
//! for one **sort key** per string — bytes that compare as the locale orders
//! the strings — and sorts by those, as it sorts everything else. So the
//! tables a collation needs, some hundred kilobytes, are not part of the
//! engine:
//!
//! - With the feature `icu` the collator is [`Icu`], ICU4X with the data it
//!   was compiled with. A server and the tests build that way.
//! - Without it — the engine module of the browser — a collator has to be
//!   [`install`]ed; the page loads it as a module of its own when a query
//!   first names a locale. A sort by a locale without one is refused.
//!
//! A sort key says how the collation orders two strings, and calls strings
//! equal that differ in what it ignores. The engine orders those by their
//! bytes, which is what a deterministic PostgreSQL collation does.

use std::cell::RefCell;
use std::rc::Rc;

use opengrid_columns::Column;
use opengrid_columns::sort::SortKeys;

/// Makes the sort keys of a locale.
pub trait Collator {
    /// Appends to `keys` one key per text, in the order of `texts`.
    fn sort_keys(
        &self,
        locale: &str,
        texts: &mut dyn Iterator<Item = &str>,
        keys: &mut SortKeys,
    ) -> Result<(), String>;
}

thread_local! {
    static INSTALLED: RefCell<Option<Rc<dyn Collator>>> = const { RefCell::new(None) };
}

/// Makes `collator` the one this thread's queries sort by; it replaces
/// [`Icu`] where the feature is on. The engine module of the browser runs on
/// one thread, and that is where this is for.
pub fn install(collator: Rc<dyn Collator>) {
    INSTALLED.with(|installed| *installed.borrow_mut() = Some(collator));
}

/// Whether a sort by a locale can be answered here.
pub fn available() -> bool {
    cfg!(feature = "icu") || INSTALLED.with(|installed| installed.borrow().is_some())
}

/// One key per row of a string column; the row of a NULL gets the key of the
/// empty string, which nothing reads.
pub(crate) fn sort_keys(locale: &str, column: &Column) -> Result<SortKeys, String> {
    let rows = column.len();
    let mut texts = (0..rows).map(|row| {
        if column.is_null(row) {
            ""
        } else {
            column.str(row)
        }
    });
    let mut keys = SortKeys::with_capacity(rows);
    let installed = INSTALLED.with(|installed| installed.borrow().clone());
    match installed {
        Some(collator) => collator.sort_keys(locale, &mut texts, &mut keys)?,
        None => built_in(locale, &mut texts, &mut keys)?,
    }
    if keys.len() != rows {
        return Err(format!(
            "the collator made {} keys for {rows} rows",
            keys.len()
        ));
    }
    Ok(keys)
}

#[cfg(feature = "icu")]
fn built_in(
    locale: &str,
    texts: &mut dyn Iterator<Item = &str>,
    keys: &mut SortKeys,
) -> Result<(), String> {
    Icu.sort_keys(locale, texts, keys)
}

#[cfg(not(feature = "icu"))]
fn built_in(
    locale: &str,
    _texts: &mut dyn Iterator<Item = &str>,
    _keys: &mut SortKeys,
) -> Result<(), String> {
    Err(format!(
        "a sort by the collation {locale:?} needs a collator, and none is loaded"
    ))
}

/// ICU4X with its compiled data: every CLDR locale, Han in code point order.
/// A locale it has no rules for sorts by its parent's, in the end the root's.
#[cfg(feature = "icu")]
pub struct Icu;

#[cfg(feature = "icu")]
impl Icu {
    /// How this collator orders two strings of a locale: by the collation,
    /// then by bytes — the order the engine's sort has, as one comparison. A
    /// connector gives it to a database that has no collations of its own.
    pub fn comparison(
        locale: &str,
    ) -> Result<impl Fn(&str, &str) -> std::cmp::Ordering + Send + Sync + 'static, String> {
        let collator = collator(locale)?;
        Ok(move |a: &str, b: &str| {
            collator
                .compare(a, b)
                .then_with(|| a.as_bytes().cmp(b.as_bytes()))
        })
    }
}

#[cfg(feature = "icu")]
fn collator(locale: &str) -> Result<icu_collator::CollatorBorrowed<'static>, String> {
    let parsed: icu_locale_core::Locale = locale
        .parse()
        .map_err(|error| format!("collation {locale:?}: {error}"))?;
    icu_collator::Collator::try_new((&parsed).into(), Default::default())
        .map_err(|error| format!("collation {locale:?}: {error}"))
}

#[cfg(feature = "icu")]
impl Collator for Icu {
    fn sort_keys(
        &self,
        locale: &str,
        texts: &mut dyn Iterator<Item = &str>,
        keys: &mut SortKeys,
    ) -> Result<(), String> {
        let collator = collator(locale)?;
        let mut key = Vec::new();
        for text in texts {
            key.clear();
            let Ok(()) = collator.write_sort_key_to(text, &mut key);
            keys.push(&key);
        }
        Ok(())
    }
}
