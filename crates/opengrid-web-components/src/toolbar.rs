//! The page's own items in the toolbar (issue #531): `set_toolbar`.
//!
//! Checked like `set_columns`: every problem of a call at once, and a call with
//! a problem applies none of it. The reading of the JavaScript values is the
//! element's; this module holds what the page asked for and what is wrong with
//! it, so both are testable without a browser.

use std::collections::HashSet;

/// How the page gave one value: not at all, as the wrong kind of value, or as
/// it should be.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Given<T> {
    #[default]
    Absent,
    Wrong,
    Is(T),
}

/// One entry of a menu, before it is checked: `{ id, label }`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RawEntry {
    pub id: Given<String>,
    pub label: Given<String>,
    /// The keys the entry has that it should not.
    pub unknown: Vec<String>,
}

/// One item of a `set_toolbar` call, before it is checked.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RawItem {
    pub id: Given<String>,
    pub label: Given<String>,
    pub disabled: Given<bool>,
    pub reason: Given<String>,
    /// `Wrong`: not a list. An entry that is no object is `None`.
    pub items: Given<Vec<Option<RawEntry>>>,
    pub unknown: Vec<String>,
}

/// An entry of a menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub label: String,
}

/// An item the grid draws: a button, or — with `entries` — a menu button; with
/// `disabled` it cannot act, and the reason says why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub id: String,
    pub label: String,
    pub entries: Option<Vec<Entry>>,
    pub disabled: Option<String>,
}

/// What is wrong with a call, in words: the item it is about and the matter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub subject: String,
    pub reason: String,
}

impl Problem {
    fn new(subject: &str, reason: impl Into<String>) -> Self {
        Self {
            subject: subject.to_owned(),
            reason: reason.into(),
        }
    }

    /// `subject: reason`, as the status line says it.
    pub fn sentence(&self) -> String {
        format!("{}: {}", self.subject, self.reason)
    }
}

/// The keys an item may have, and an entry.
const ITEM_KEYS: &str = "id, label, items, disabled, reason";
const ENTRY_KEYS: &str = "id, label";

/// A text the item has to have: present, a string, and something.
fn text(
    given: &Given<String>,
    what: &str,
    subject: &str,
    problems: &mut Vec<Problem>,
) -> Option<String> {
    match given {
        Given::Absent => problems.push(Problem::new(subject, format!("has no {what}"))),
        Given::Wrong => problems.push(Problem::new(subject, format!("{what} is not a string"))),
        Given::Is(value) if value.trim().is_empty() => {
            problems.push(Problem::new(subject, format!("has an empty {what}")));
        }
        Given::Is(value) => return Some(value.clone()),
    }
    None
}

/// An id, and the check that no other item or entry of the call has it.
fn id_of(
    given: &Given<String>,
    subject: &str,
    seen: &mut HashSet<String>,
    problems: &mut Vec<Problem>,
) -> Option<String> {
    let id = text(given, "id", subject, problems)?;
    if !seen.insert(id.clone()) {
        problems.push(Problem::new(&id, "id is used twice"));
    }
    Some(id)
}

fn unknown_keys(keys: &[String], known: &str, subject: &str, problems: &mut Vec<Problem>) {
    for key in keys {
        problems.push(Problem::new(
            subject,
            format!("unknown key \"{key}\" (known: {known})"),
        ));
    }
}

/// Checks a whole call. Answers the items, or **every** problem at once.
pub fn validate(raw: &[Option<RawItem>]) -> Result<Vec<Item>, Vec<Problem>> {
    let mut problems = Vec::new();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (at, item) in raw.iter().enumerate() {
        let place = format!("item {}", at + 1);
        let Some(item) = item else {
            problems.push(Problem::new(&place, "is not an object"));
            continue;
        };
        let subject = match &item.id {
            Given::Is(id) if !id.trim().is_empty() => id.clone(),
            _ => place,
        };
        let id = id_of(&item.id, &subject, &mut seen, &mut problems);
        let label = text(&item.label, "label", &subject, &mut problems);
        unknown_keys(&item.unknown, ITEM_KEYS, &subject, &mut problems);

        let disabled = match (&item.disabled, &item.reason) {
            (Given::Wrong, _) => {
                problems.push(Problem::new(&subject, "disabled is not true or false"));
                None
            }
            (Given::Is(true), reason) => {
                let reason = match reason {
                    Given::Is(reason) if !reason.trim().is_empty() => Some(reason.clone()),
                    Given::Wrong => {
                        problems.push(Problem::new(&subject, "reason is not a string"));
                        None
                    }
                    _ => {
                        problems.push(Problem::new(&subject, "is disabled without a reason"));
                        None
                    }
                };
                reason.map(Some)
            }
            (_, Given::Absent) => Some(None),
            (_, Given::Wrong) => {
                problems.push(Problem::new(&subject, "reason is not a string"));
                None
            }
            (_, Given::Is(_)) => {
                problems.push(Problem::new(&subject, "has a reason but is not disabled"));
                None
            }
        };

        let entries = match &item.items {
            Given::Absent => Some(None),
            Given::Wrong => {
                problems.push(Problem::new(&subject, "items is not a list"));
                None
            }
            Given::Is(list) if list.is_empty() => {
                problems.push(Problem::new(&subject, "has no entries"));
                None
            }
            Given::Is(list) => {
                let mut entries = Vec::new();
                let mut whole = true;
                for (index, entry) in list.iter().enumerate() {
                    let place = format!("{subject} entry {}", index + 1);
                    let Some(entry) = entry else {
                        problems.push(Problem::new(&place, "is not an object"));
                        whole = false;
                        continue;
                    };
                    let name = match &entry.id {
                        Given::Is(id) if !id.trim().is_empty() => id.clone(),
                        _ => place,
                    };
                    let id = id_of(&entry.id, &name, &mut seen, &mut problems);
                    let label = text(&entry.label, "label", &name, &mut problems);
                    unknown_keys(&entry.unknown, ENTRY_KEYS, &name, &mut problems);
                    match (id, label) {
                        (Some(id), Some(label)) if entry.unknown.is_empty() => {
                            entries.push(Entry { id, label });
                        }
                        _ => whole = false,
                    }
                }
                whole.then_some(Some(entries))
            }
        };

        if let (Some(id), Some(label), Some(disabled), Some(entries)) =
            (id, label, disabled, entries)
            && item.unknown.is_empty()
        {
            out.push(Item {
                id,
                label,
                entries,
                disabled,
            });
        }
    }
    if problems.is_empty() {
        Ok(out)
    } else {
        Err(problems)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is(text: &str) -> Given<String> {
        Given::Is(text.to_owned())
    }

    fn button(id: &str, label: &str) -> Option<RawItem> {
        Some(RawItem {
            id: is(id),
            label: is(label),
            ..RawItem::default()
        })
    }

    fn entry(id: &str, label: &str) -> Option<RawEntry> {
        Some(RawEntry {
            id: is(id),
            label: is(label),
            unknown: Vec::new(),
        })
    }

    fn refused(raw: &[Option<RawItem>]) -> Vec<String> {
        validate(raw)
            .expect_err("is refused")
            .iter()
            .map(Problem::sentence)
            .collect()
    }

    #[test]
    fn a_button_a_menu_and_a_disabled_item_are_accepted_in_order() {
        let menu = Some(RawItem {
            items: Given::Is(vec![entry("pdf", "PDF"), entry("xlsx", "Excel")]),
            ..button("export", "Export").unwrap()
        });
        let off = Some(RawItem {
            disabled: Given::Is(true),
            reason: is("Nothing selected"),
            ..button("bulk", "Bulk change").unwrap()
        });
        let items = validate(&[button("prepare", "Prepare"), menu, off]).expect("accepted");
        assert_eq!(
            items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["prepare", "export", "bulk"]
        );
        assert_eq!(items[0].entries, None);
        assert_eq!(items[0].disabled, None);
        assert_eq!(items[1].entries.as_ref().map(Vec::len), Some(2));
        assert_eq!(items[2].disabled.as_deref(), Some("Nothing selected"));
    }

    #[test]
    fn nothing_is_nothing_to_draw() {
        assert_eq!(validate(&[]), Ok(Vec::new()));
        let off = Some(RawItem {
            disabled: Given::Is(false),
            ..button("a", "A").unwrap()
        });
        assert!(validate(&[off]).is_ok());
    }

    #[test]
    fn a_disabled_menu_keeps_its_entries() {
        let off = Some(RawItem {
            disabled: Given::Is(true),
            reason: is("No rows"),
            items: Given::Is(vec![entry("pdf", "PDF")]),
            ..button("export", "Export").unwrap()
        });
        let items = validate(&[off]).expect("accepted");
        assert_eq!(items[0].entries.as_ref().map(Vec::len), Some(1));
        assert_eq!(items[0].disabled.as_deref(), Some("No rows"));
    }

    #[test]
    fn an_unknown_key_is_named() {
        let odd = Some(RawItem {
            unknown: vec!["position".to_owned()],
            ..button("a", "A").unwrap()
        });
        let problems = refused(&[odd]);
        assert_eq!(problems.len(), 1);
        assert!(
            problems[0].starts_with("a: unknown key \"position\""),
            "{problems:?}"
        );
    }

    #[test]
    fn an_unknown_key_of_an_entry_is_named_with_the_entry() {
        let odd = Some(RawEntry {
            unknown: vec!["disabled".to_owned()],
            ..entry("pdf", "PDF").unwrap()
        });
        let menu = Some(RawItem {
            items: Given::Is(vec![odd]),
            ..button("export", "Export").unwrap()
        });
        let problems = refused(&[menu]);
        assert!(
            problems[0].starts_with("pdf: unknown key \"disabled\""),
            "{problems:?}"
        );
    }

    #[test]
    fn empty_and_missing_ids_and_labels_are_reported() {
        let raw = [
            button("a", "  "),
            button("", "B"),
            Some(RawItem {
                label: is("C"),
                ..RawItem::default()
            }),
            Some(RawItem {
                id: Given::Wrong,
                label: Given::Wrong,
                ..RawItem::default()
            }),
        ];
        assert_eq!(
            refused(&raw),
            [
                "a: has an empty label",
                "item 2: has an empty id",
                "item 3: has no id",
                "item 4: id is not a string",
                "item 4: label is not a string",
            ]
        );
    }

    #[test]
    fn a_duplicate_id_is_reported_across_items_and_entries() {
        let menu = Some(RawItem {
            items: Given::Is(vec![
                entry("a", "Again"),
                entry("pdf", "PDF"),
                entry("pdf", "PDF 2"),
            ]),
            ..button("export", "Export").unwrap()
        });
        assert_eq!(
            refused(&[button("a", "A"), menu]),
            ["a: id is used twice", "pdf: id is used twice"]
        );
    }

    #[test]
    fn disabled_needs_its_reason_and_a_reason_needs_disabled() {
        let silent = Some(RawItem {
            disabled: Given::Is(true),
            ..button("a", "A").unwrap()
        });
        let blank = Some(RawItem {
            disabled: Given::Is(true),
            reason: is(" "),
            ..button("b", "B").unwrap()
        });
        let stray = Some(RawItem {
            reason: is("why"),
            ..button("c", "C").unwrap()
        });
        let wrong = Some(RawItem {
            disabled: Given::Wrong,
            ..button("d", "D").unwrap()
        });
        assert_eq!(
            refused(&[silent, blank, stray, wrong]),
            [
                "a: is disabled without a reason",
                "b: is disabled without a reason",
                "c: has a reason but is not disabled",
                "d: disabled is not true or false",
            ]
        );
    }

    #[test]
    fn a_menu_without_entries_or_with_a_broken_one_is_refused() {
        let empty = Some(RawItem {
            items: Given::Is(Vec::new()),
            ..button("a", "A").unwrap()
        });
        let not_a_list = Some(RawItem {
            items: Given::Wrong,
            ..button("b", "B").unwrap()
        });
        let broken = Some(RawItem {
            items: Given::Is(vec![None, Some(RawEntry::default())]),
            ..button("c", "C").unwrap()
        });
        assert_eq!(
            refused(&[empty, not_a_list, broken]),
            [
                "a: has no entries",
                "b: items is not a list",
                "c entry 1: is not an object",
                "c entry 2: has no id",
                "c entry 2: has no label",
            ]
        );
    }

    #[test]
    fn an_item_that_is_no_object_is_reported_by_its_place() {
        assert_eq!(refused(&[None]), ["item 1: is not an object"]);
    }

    #[test]
    fn one_bad_item_applies_none_of_the_call() {
        assert!(validate(&[button("a", "A"), button("b", "")]).is_err());
    }
}
