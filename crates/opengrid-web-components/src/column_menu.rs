//! The column menu (plan point 64): what it offers, as data.
//!
//! # A door, not a room
//!
//! Everything in this menu can be done without it: sorting from the header
//! (`Enter`), filtering in the filter row, hiding in the column list, grouping
//! through `group-by` or the view, aggregates through `set_columns` or the view.
//! That is the argument WCAG 2.5.7 asks for, turned around: the menu is a
//! second way in, built once the rooms stood.
//!
//! # Why the menu holds no form
//!
//! The prototype puts the filter's operator and value into the menu. A `menu`
//! may only contain menu items; a `<select>` inside one breaks the role, and a
//! screen reader that entered a menu does not expect to type. So "Filter …"
//! *jumps* to the column's field in the filter row, where filtering is already
//! accessible (point 48), and the menu stays a menu.
//!
//! # Only what does something
//!
//! An entry that would do nothing is not shown: no aggregate choice on a text
//! column (only `count` would fit, and "count" in every text header is noise),
//! no grouping by a decimal, no "hide" on the last visible column.

use opengrid_types::DataType;

use crate::presentation::aggregates_for;
use crate::texts::GridTexts;

/// One entry of the menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    /// A `menuitem`, or a `menuitemradio` when `checked` is `Some`.
    Item {
        /// What activating it does, as the element reads it back.
        action: String,
        label: String,
        checked: Option<bool>,
    },
    /// A `separator`.
    Separator,
    /// A labelled `group` of entries.
    Group { label: String, entries: Vec<Entry> },
}

impl Entry {
    fn item(action: impl Into<String>, label: &str) -> Self {
        Entry::Item {
            action: action.into(),
            label: label.to_owned(),
            checked: None,
        }
    }

    fn radio(action: impl Into<String>, label: &str, checked: bool) -> Self {
        Entry::Item {
            action: action.into(),
            label: label.to_owned(),
            checked: Some(checked),
        }
    }
}

/// What the menu needs to know about its column and the grid.
pub struct Column<'a> {
    pub name: &'a str,
    pub data_type: DataType,
    /// `Some("asc" | "desc")` when this column is the sort.
    pub sort: Option<&'a str>,
    /// The aggregate this column shows in groups.
    pub aggregate: Option<crate::presentation::Summary>,
    /// The columns grouped by now.
    pub group_by: &'a [String],
    /// Whether grouping is possible at all — not while paging (point 62).
    pub can_group: bool,
    /// How many columns are visible.
    pub visible: usize,
}

/// The entries of one column's menu, in order.
pub fn entries(column: &Column<'_>, texts: &GridTexts) -> Vec<Entry> {
    let mut out = vec![
        Entry::radio(
            "sort:asc",
            &texts.sort_ascending,
            column.sort == Some("asc"),
        ),
        Entry::radio(
            "sort:desc",
            &texts.sort_descending,
            column.sort == Some("desc"),
        ),
        Entry::Separator,
        Entry::item("filter", &texts.filter_column),
    ];

    let allowed = aggregates_for(column.data_type);
    if allowed.len() > 1 {
        let mut group = vec![Entry::radio(
            "aggregate:none",
            &texts.aggregate_none,
            column.aggregate.is_none(),
        )];
        for function in allowed {
            group.push(Entry::radio(
                format!("aggregate:{}", function.as_str()),
                texts.aggregate_name(function),
                column.aggregate == Some(function),
            ));
        }
        out.push(Entry::Separator);
        out.push(Entry::Group {
            label: texts.aggregate_group.clone(),
            entries: group,
        });
    }

    let mut rest = Vec::new();
    if column.can_group && crate::grouping::groupable(column.data_type) {
        if column.group_by.iter().any(|name| name == column.name) {
            rest.push(Entry::item("group:remove", &texts.ungroup_column));
        } else {
            rest.push(Entry::item("group:first", &texts.group_by_column));
            if !column.group_by.is_empty() {
                rest.push(Entry::item("group:second", &texts.group_second_level));
            }
        }
    }
    if column.visible > 1 {
        rest.push(Entry::item("hide", &texts.hide_column));
    }
    if !rest.is_empty() {
        out.push(Entry::Separator);
        out.extend(rest);
    }
    out
}

/// The grouping a `group:*` action leaves behind.
///
/// `first` makes this column the only level; `second` keeps the outermost level
/// and puts this column under it; `remove` takes it out. At most two levels,
/// as `group-by` allows.
pub fn regrouped(action: &str, column: &str, group_by: &[String]) -> Vec<String> {
    match action {
        "group:first" => vec![column.to_owned()],
        "group:second" => {
            let mut out: Vec<String> = group_by.first().cloned().into_iter().collect();
            if out.first().map(String::as_str) != Some(column) {
                out.push(column.to_owned());
            }
            out
        }
        "group:remove" => group_by
            .iter()
            .filter(|name| name.as_str() != column)
            .cloned()
            .collect(),
        _ => group_by.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actions(entries: &[Entry]) -> Vec<String> {
        let mut out = Vec::new();
        for entry in entries {
            match entry {
                Entry::Item { action, .. } => out.push(action.clone()),
                Entry::Group { entries, .. } => out.extend(actions(entries)),
                Entry::Separator => {}
            }
        }
        out
    }

    fn column<'a>(name: &'a str, data_type: DataType, group_by: &'a [String]) -> Column<'a> {
        Column {
            name,
            data_type,
            sort: None,
            aggregate: None,
            group_by,
            can_group: true,
            visible: 5,
        }
    }

    #[test]
    fn a_number_offers_its_aggregates_and_no_grouping() {
        let texts = GridTexts::default();
        let got = actions(&entries(&column("amount", DataType::Float64, &[]), &texts));
        assert_eq!(
            got,
            [
                "sort:asc",
                "sort:desc",
                "filter",
                "aggregate:none",
                "aggregate:count",
                "aggregate:sum",
                "aggregate:avg",
                "aggregate:min",
                "aggregate:max",
                "aggregate:range",
                "hide",
            ]
        );
    }

    #[test]
    fn text_offers_grouping_and_no_aggregate() {
        // "count" in every text header would be noise, and it is the only
        // aggregate text allows.
        let texts = GridTexts::default();
        let got = actions(&entries(&column("customer", DataType::Utf8, &[]), &texts));
        assert_eq!(
            got,
            ["sort:asc", "sort:desc", "filter", "group:first", "hide"]
        );
    }

    #[test]
    fn a_grouped_column_offers_to_ungroup_and_another_a_second_level() {
        let texts = GridTexts::default();
        let by = ["country".to_owned()];
        assert!(
            actions(&entries(&column("country", DataType::Utf8, &by), &texts))
                .contains(&"group:remove".to_owned())
        );
        let other = actions(&entries(&column("customer", DataType::Utf8, &by), &texts));
        assert!(other.contains(&"group:first".to_owned()));
        assert!(other.contains(&"group:second".to_owned()));
    }

    #[test]
    fn nothing_that_would_do_nothing() {
        let texts = GridTexts::default();
        let mut last = column("customer", DataType::Utf8, &[]);
        last.visible = 1;
        last.can_group = false;
        let got = actions(&entries(&last, &texts));
        // No grouping while paging, and the last column cannot be hidden.
        assert_eq!(got, ["sort:asc", "sort:desc", "filter"]);
    }

    #[test]
    fn the_active_choices_are_checked() {
        let texts = GridTexts::default();
        let mut amount = column("amount", DataType::Int64, &[]);
        amount.sort = Some("desc");
        amount.aggregate = Some(crate::presentation::Summary::Fn(
            opengrid_query::AggregateFn::Sum,
        ));
        let got = entries(&amount, &texts);
        let checked: Vec<String> = got
            .iter()
            .flat_map(|entry| match entry {
                Entry::Group { entries, .. } => entries.clone(),
                other => vec![other.clone()],
            })
            .filter_map(|entry| match entry {
                Entry::Item {
                    action,
                    checked: Some(true),
                    ..
                } => Some(action),
                _ => None,
            })
            .collect();
        assert_eq!(checked, ["sort:desc", "aggregate:sum"]);
    }

    #[test]
    fn regrouping_keeps_at_most_two_levels() {
        let by = vec!["country".to_owned(), "customer".to_owned()];
        assert_eq!(regrouped("group:first", "region", &by), ["region"]);
        assert_eq!(
            regrouped("group:second", "region", &by),
            ["country", "region"]
        );
        assert_eq!(regrouped("group:remove", "country", &by), ["customer"]);
        // Asking the outermost level to be its own second level changes nothing.
        assert_eq!(regrouped("group:second", "country", &by[..1]), ["country"]);
    }
}
