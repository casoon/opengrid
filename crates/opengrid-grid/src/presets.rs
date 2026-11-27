//! Presets (issue #532): the page's named views — each a filter the page
//! knows, one of them always chosen.
//!
//! A preset is **not** the reader's filter. The page defines it, the reader
//! picks one, and the chosen one is and-ed onto the page's `filter` and the
//! reader's filters in every query the grid sends. It is in the view (`preset`)
//! and it is not a chip: "Remove all" takes back what the reader added, and the
//! preset is the view they are looking at.
//!
//! A preset may have no filter (an `All`), and the first preset is the default.

use opengrid_json::{FromJson, Json};
use opengrid_query::FilterExpr;

/// One named view of the page's.
#[derive(Clone, Debug, PartialEq)]
pub struct Preset {
    /// What the view names it by (`preset` in the view); unique in a list.
    pub id: String,
    /// What the reader sees and a screen reader says.
    pub label: String,
    /// The rows of the view; none is every row.
    pub filter: Option<FilterExpr>,
}

/// The keys an entry may have: a typo is said, not ignored.
const KEYS: [&str; 3] = ["id", "label", "filter"];

/// Reads the page's list — `[{ id, label, filter? }]`, the filter in the
/// wire's notation.
///
/// **All or nothing**: answers the presets, or every problem at once, one
/// sentence each. A list that is refused is applied not at all.
pub fn parse(value: &Json) -> Result<Vec<Preset>, Vec<String>> {
    let Some(entries) = value.as_array() else {
        return Err(vec!["presets is not a list".to_owned()]);
    };
    let mut presets: Vec<Preset> = Vec::with_capacity(entries.len());
    let mut problems = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let at = format!("presets[{index}]");
        let Some(object) = entry.as_object() else {
            problems.push(format!("{at} is not an object"));
            continue;
        };
        for key in object.keys().filter(|key| !KEYS.contains(&key.as_str())) {
            problems.push(format!("{at}: {key} is not a field of a preset"));
        }
        let text = |name: &str| {
            object
                .get(name)
                .and_then(Json::as_str)
                .filter(|text| !text.trim().is_empty())
        };
        let id = text("id");
        if id.is_none() {
            problems.push(format!("{at} has no id"));
        }
        let label = text("label");
        if label.is_none() {
            problems.push(format!("{at} has no label"));
        }
        let filter = match object.get("filter") {
            None => None,
            Some(tree) if tree.is_null() => None,
            Some(tree) => match FilterExpr::from_json(tree) {
                Ok(filter) => Some(filter),
                Err(error) => {
                    problems.push(format!("{at}: the filter is not a filter: {error}"));
                    continue;
                }
            },
        };
        let (Some(id), Some(label)) = (id, label) else {
            continue;
        };
        if presets.iter().any(|preset| preset.id == id) {
            problems.push(format!("{at}: {id} is the id of another preset"));
            continue;
        }
        presets.push(Preset {
            id: id.to_owned(),
            label: label.trim().to_owned(),
            filter,
        });
    }
    if problems.is_empty() {
        Ok(presets)
    } else {
        Err(problems)
    }
}

/// The presets a grid holds and the one chosen.
///
/// The choice is an id and is kept even where no preset of that id is held: a
/// view applied before the page set its presets names one, and the list that
/// follows finds it again ([`Presets::replace`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Presets {
    items: Vec<Preset>,
    chosen: Option<String>,
}

impl Presets {
    /// The presets, in the page's order.
    pub fn items(&self) -> &[Preset] {
        &self.items
    }

    /// The id of the chosen preset: the one the view names.
    pub fn chosen(&self) -> Option<&str> {
        self.chosen.as_deref()
    }

    /// The chosen preset, if the page has one of that id.
    pub fn active(&self) -> Option<&Preset> {
        let id = self.chosen.as_deref()?;
        self.items.iter().find(|preset| preset.id == id)
    }

    /// The chosen preset's filter: none for an `All`, and where nothing is chosen.
    pub fn filter(&self) -> Option<FilterExpr> {
        self.active()?.filter.clone()
    }

    /// Takes a new list. What was chosen stays chosen where the new list still
    /// has it; otherwise the first preset is — the default.
    pub fn replace(&mut self, items: Vec<Preset>) {
        self.items = items;
        if self.active().is_none() {
            self.chosen = self.items.first().map(|preset| preset.id.clone());
        }
    }

    /// Chooses `id`; false, and nothing changed, where the page has none of that id.
    pub fn choose(&mut self, id: &str) -> bool {
        if !self.items.iter().any(|preset| preset.id == id) {
            return false;
        }
        self.chosen = Some(id.to_owned());
        true
    }

    /// What a view's `preset` comes to here, for the view to be applied: none
    /// is the first preset (the default); an id is kept as it is — and, where
    /// the page has presets, it must be one of them, or the view is refused
    /// like one naming a column the grid does not have. Where the page has not
    /// set any yet the id waits for them.
    pub fn resolve(&self, wanted: Option<&str>) -> Result<Option<String>, String> {
        let Some(first) = self.items.first() else {
            return Ok(wanted.map(str::to_owned));
        };
        match wanted {
            None => Ok(Some(first.id.clone())),
            Some(id) if self.items.iter().any(|preset| preset.id == id) => Ok(Some(id.to_owned())),
            Some(id) => Err(format!("preset: {id} is not a preset of this grid")),
        }
    }

    /// Takes the choice of a view that [`resolve`](Self::resolve) accepted.
    pub fn restore(&mut self, chosen: Option<String>) {
        self.chosen = chosen;
    }
}

/// The filters a query runs under, and-ed in the order given: the page's, the
/// chosen preset's, the reader's. One of them is that one; none is no filter.
pub fn all_of(parts: impl IntoIterator<Item = Option<FilterExpr>>) -> Option<FilterExpr> {
    let mut parts: Vec<FilterExpr> = parts.into_iter().flatten().collect();
    match parts.len() {
        0 | 1 => parts.pop(),
        _ => Some(FilterExpr::And(parts)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str) -> Result<Vec<Preset>, Vec<String>> {
        parse(&Json::parse(text).expect("json"))
    }

    fn presets() -> Presets {
        let mut presets = Presets::default();
        presets.replace(
            read(
                r#"[
                    { "id": "all", "label": "All" },
                    { "id": "dev", "label": "Deviating",
                      "filter": { "field": "price", "op": "ne", "value": 1 } }
                ]"#,
            )
            .expect("a list"),
        );
        presets
    }

    #[test]
    fn the_first_preset_is_the_default_and_a_kept_choice_survives_a_new_list() {
        let mut held = presets();
        assert_eq!(held.chosen(), Some("all"));
        assert_eq!(held.filter(), None);
        assert!(held.choose("dev"));
        assert!(held.filter().is_some());
        // The same ids again: still "dev".
        held.replace(held.items().to_vec());
        assert_eq!(held.chosen(), Some("dev"));
        // A list without it: the default.
        held.replace(vec![held.items()[0].clone()]);
        assert_eq!(held.chosen(), Some("all"));
        // No presets: nothing chosen.
        held.replace(Vec::new());
        assert_eq!(held.chosen(), None);
        assert_eq!(held.filter(), None);
    }

    #[test]
    fn choosing_what_the_page_does_not_have_changes_nothing() {
        let mut held = presets();
        assert!(!held.choose("nope"));
        assert_eq!(held.chosen(), Some("all"));
    }

    #[test]
    fn a_view_names_a_preset_the_page_has_or_is_refused() {
        let held = presets();
        assert_eq!(held.resolve(None), Ok(Some("all".to_owned())));
        assert_eq!(held.resolve(Some("dev")), Ok(Some("dev".to_owned())));
        let refused = held.resolve(Some("nope")).expect_err("refused");
        assert_eq!(refused, "preset: nope is not a preset of this grid");
        // Before the page sets presets the id waits for them.
        let mut empty = Presets::default();
        assert_eq!(empty.resolve(Some("dev")), Ok(Some("dev".to_owned())));
        assert_eq!(empty.resolve(None), Ok(None));
        empty.restore(Some("dev".to_owned()));
        empty.replace(held.items().to_vec());
        assert_eq!(empty.chosen(), Some("dev"));
    }

    #[test]
    fn the_filters_are_anded_in_the_order_page_preset_reader() {
        let f = |value: i64| FilterExpr::Cmp {
            field: opengrid_types::FieldName::new("a").unwrap(),
            op: opengrid_query::CmpOp::Eq,
            value: Json::from(value),
        };
        assert_eq!(all_of([None, None, None]), None);
        assert_eq!(all_of([Some(f(1)), None, None]), Some(f(1)));
        assert_eq!(all_of([None, Some(f(2)), None]), Some(f(2)));
        // Two keep the shape the page's filter and the reader's always had.
        assert_eq!(
            all_of([Some(f(1)), None, Some(f(3))]),
            Some(FilterExpr::And(vec![f(1), f(3)]))
        );
        assert_eq!(
            all_of([Some(f(1)), Some(f(2)), Some(f(3))]),
            Some(FilterExpr::And(vec![f(1), f(2), f(3)]))
        );
    }

    #[test]
    fn a_list_reads_in_order_and_a_preset_needs_no_filter() {
        let presets = read(
            r#"[
                { "id": "all", "label": "All active" },
                { "id": "deviating", "label": "Deviating",
                  "filter": { "field": "price", "op": "ne", "value": 1 } },
                { "id": "none", "label": "Hidden", "filter": null }
            ]"#,
        )
        .expect("a list");
        assert_eq!(
            presets.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            ["all", "deviating", "none"]
        );
        assert_eq!(presets[0].filter, None);
        assert!(presets[1].filter.is_some());
        assert_eq!(presets[2].filter, None);
    }

    #[test]
    fn an_empty_list_is_no_presets() {
        assert_eq!(read("[]"), Ok(Vec::new()));
    }

    #[test]
    fn every_problem_is_said_and_none_of_the_list_is_kept() {
        let problems = read(
            r#"[
                { "id": "a", "label": "A" },
                { "id": "a", "label": "Again" },
                { "label": "No id" },
                { "id": "c" },
                { "id": "d", "label": "D", "filter": { "field": "x" } },
                { "id": "e", "label": "E", "flter": null },
                { "id": "f", "label": "  " },
                7
            ]"#,
        )
        .expect_err("refused");
        assert_eq!(problems.len(), 7, "{problems:?}");
        assert!(
            problems
                .iter()
                .any(|p| p.contains("presets[1]") && p.contains("another"))
        );
        assert!(
            problems
                .iter()
                .any(|p| p.contains("presets[2]") && p.contains("no id"))
        );
        assert!(
            problems
                .iter()
                .any(|p| p.contains("presets[3]") && p.contains("no label"))
        );
        assert!(
            problems
                .iter()
                .any(|p| p.contains("presets[4]") && p.contains("not a filter"))
        );
        assert!(
            problems
                .iter()
                .any(|p| p.contains("presets[5]") && p.contains("flter"))
        );
        // A label of blanks is none.
        assert!(
            problems
                .iter()
                .any(|p| p.contains("presets[6]") && p.contains("no label"))
        );
        assert!(
            problems
                .iter()
                .any(|p| p.contains("presets[7]") && p.contains("not an object"))
        );
    }

    #[test]
    fn what_is_not_a_list_is_refused() {
        assert!(read(r#"{ "id": "a", "label": "A" }"#).is_err());
        assert!(read("\"a\"").is_err());
    }
}
