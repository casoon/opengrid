//! Grouping (plan point 62): the flat display list, as arithmetic.
//!
//! # Why this module is the whole point
//!
//! Point 38 declared paging and virtualization mutually exclusive — "a window
//! inside a window, with two truths about `aria-rowcount`". Grouping smells the
//! same: expanding a group changes how many rows there are and where every row
//! after it sits. It was decided (F1, 2026-09-23) that grouping stays
//! virtualized anyway, and this module is the reason that is possible:
//!
//! > The group query answers every group's **row count**. From the counts and
//! > the set of expanded groups, the flat display list is pure arithmetic.
//!
//! So there is still exactly **one** truth about `aria-rowcount` — it is
//! computed rather than counted. A position maps to "the header of group X" or
//! "row *n* of group X", and the rows of a visible window are fetched per group
//! with `filter` on the group key plus `offset`/`limit`. Nothing here holds a
//! row of data; it holds counts.
//!
//! # The display list **is** the logical row space
//!
//! The grid's focus, window, pool and `aria-rowindex` all work on logical rows.
//! Under grouping, a logical row is a *position in the display list* — group
//! header or data row alike. That is what lets the renderer, the virtualization
//! and the keyboard stay as they are: they never learn that some positions are
//! headers, except the one place that draws them.
//!
//! # Keys
//!
//! A group key is the **wire JSON** of the key value (E13), because that is what
//! a filter literal is made of and what a view (point 59) can store. NULL and
//! the empty string are different groups (S10, S14) and filter differently:
//! NULL through `is_null` — `eq null` would be *unknown* and match nothing (S1)
//! — the empty string through `eq ""`.

use std::collections::BTreeSet;

use opengrid_query::{CmpOp, FilterExpr};

use crate::presentation::Summary;
use opengrid_types::{DataType, FieldName, Value};
use serde_json::Value as Json;

/// The most levels a grid groups by. The prototype has two, and a third would
/// need a tree in the UI that nobody has designed (point 62 §Nicht Teil).
pub const MAX_LEVELS: usize = 2;

/// The alias the group queries count rows under.
///
/// Prefixed so it cannot collide with a column a page is likely to have; a
/// valid identifier (`^[A-Za-z_]…`), so it passes the query validator.
pub const COUNT_ALIAS: &str = "__og_rows";

/// Whether a column may be grouped by.
///
/// Grouping by a decimal, a float or a timestamp produces one group per
/// distinct value — practically one per row, which is a list with extra steps
/// rather than a grouping. Text, booleans, whole numbers and dates have values
/// that repeat.
pub fn groupable(data_type: DataType) -> bool {
    matches!(
        data_type,
        DataType::Utf8 | DataType::Bool | DataType::Int64 | DataType::Date
    )
}

/// Reads `group-by`: a trimmed, de-duplicated list of at most [`MAX_LEVELS`].
///
/// Answers `Err` with the offending text when the list is longer — silently
/// grouping by the first two of three would look like it worked.
pub fn parse_group_by(raw: Option<&str>) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    for name in raw.unwrap_or("").split(',') {
        let name = name.trim();
        if !name.is_empty() && !out.iter().any(|known| known == name) {
            out.push(name.to_owned());
        }
    }
    if out.len() > MAX_LEVELS {
        return Err(out.join(","));
    }
    Ok(out)
}

/// The identity of a group: the canonical JSON text of each key on its path.
///
/// Text rather than `serde_json::Value`, because a path has to live in a set
/// and `Value` is neither `Ord` nor `Hash`. Canonical because
/// `Value::to_string` always writes the same value the same way.
pub type Path = Vec<String>;

/// One group of either level.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    /// The key as the wire writes it — what a filter literal is made of.
    pub key: Json,
    /// The key as a value, for the label.
    pub value: Value,
    /// Data rows in the group, across all its subgroups.
    pub count: u64,
    /// The level-2 groups, once fetched. `None` until the group is expanded for
    /// the first time on a two-level grid; always `None` on the second level
    /// and on a one-level grid.
    pub children: Option<Vec<Group>>,
    /// One value per requested aggregate (point 63), in the grouping's order.
    pub aggregates: Vec<Value>,
}

impl Group {
    pub fn new(value: Value, count: u64) -> Self {
        let key = serde_json::to_value(&value).unwrap_or(Json::Null);
        Self {
            key,
            value,
            count,
            children: None,
            aggregates: Vec::new(),
        }
    }

    fn id(&self) -> String {
        self.key.to_string()
    }
}

/// What sits at a position of the display list.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    /// A group header.
    Group {
        /// 1 or 2.
        level: usize,
        /// The keys down to this group, as JSON.
        keys: Vec<Json>,
        /// The key as a value, for the label.
        value: Value,
        /// Data rows in the group.
        count: u64,
        expanded: bool,
        /// One value per requested aggregate (point 63).
        aggregates: Vec<Value>,
    },
    /// The grand total over every matching row (point 63) — the last position
    /// of the list, so the keys and a screen reader reach it like any row.
    Total {
        /// Data rows in the whole result.
        count: u64,
        /// One value per requested aggregate.
        aggregates: Vec<Value>,
    },
    /// A data row.
    Row {
        /// The keys of the innermost group the row belongs to.
        keys: Vec<Json>,
        /// Its index inside that group, in the current sort.
        offset: u64,
    },
}

/// A slice of one group's rows that a window needs.
#[derive(Clone, Debug, PartialEq)]
pub struct Fetch {
    /// The keys of the group — the filter the rows are fetched with.
    pub keys: Vec<Json>,
    /// First row inside the group.
    pub offset: u64,
    /// How many rows.
    pub limit: u64,
    /// The display position the first of them goes to.
    pub position: u64,
}

/// The grouping state of one grid: its levels, its groups and what is open.
#[derive(Clone, Debug, Default)]
pub struct Grouping {
    /// The columns grouped by, outermost first.
    by: Vec<String>,
    /// The level-1 groups in display order. `None` until the group query
    /// answered.
    groups: Option<Vec<Group>>,
    /// Expanded groups, by path.
    expanded: BTreeSet<Path>,
    /// Display position at which each level-1 group starts, plus one entry for
    /// the end — so `starts[i+1] - starts[i]` is the size of group *i*.
    starts: Vec<u64>,
    /// The aggregates each group and the total row show, as `(column, fn)`
    /// (point 63). Empty unless a page or a reader chose some — a default
    /// "sum every number" would sum the ids.
    aggregates: Vec<(String, Summary)>,
    /// The aggregates over every matching row, once asked.
    total: Option<Vec<Value>>,
}

impl Grouping {
    /// A grouping by `by` (at most [`MAX_LEVELS`] columns), with nothing loaded
    /// and nothing open.
    pub fn new(by: Vec<String>) -> Self {
        Self {
            by,
            ..Self::default()
        }
    }

    /// The columns grouped by.
    pub fn by(&self) -> &[String] {
        &self.by
    }

    /// Number of levels, 1 or 2.
    pub fn levels(&self) -> usize {
        self.by.len()
    }

    /// Whether the group query has answered.
    pub fn is_loaded(&self) -> bool {
        self.groups.is_some()
    }

    /// Replaces the level-1 groups, keeping what was expanded.
    ///
    /// Expanded paths that no longer exist stay in the set and simply match
    /// nothing: a filter that hides a group and a filter that brings it back
    /// should find it open again, the way the reader left it.
    pub fn set_groups(&mut self, groups: Vec<Group>) {
        self.groups = Some(groups);
        self.relayout();
    }

    /// Drops the loaded groups (a filter changed), keeping what is expanded.
    pub fn invalidate(&mut self) {
        self.groups = None;
        self.total = None;
        self.starts.clear();
    }

    /// The aggregates the groups show.
    pub fn aggregates(&self) -> &[(String, Summary)] {
        &self.aggregates
    }

    /// Chooses the aggregates. A different choice makes the loaded counts stale,
    /// because they were asked together with the old aggregates.
    pub fn set_aggregates(&mut self, aggregates: Vec<(String, Summary)>) {
        if self.aggregates != aggregates {
            self.aggregates = aggregates;
            self.invalidate();
        }
    }

    /// The grand total's aggregates (point 63).
    pub fn set_total(&mut self, total: Vec<Value>) {
        self.total = Some(total);
    }

    /// Sets the level-2 groups of the level-1 group `key`.
    pub fn set_children(&mut self, key: &Json, children: Vec<Group>) {
        if let Some(group) = self
            .groups
            .as_mut()
            .and_then(|groups| groups.iter_mut().find(|group| &group.key == key))
        {
            group.children = Some(children);
        }
        self.relayout();
    }

    /// Level-1 groups that are expanded on a two-level grid but whose level-2
    /// groups have not been fetched yet — what has to be asked before the
    /// display list is complete.
    pub fn missing_children(&self) -> Vec<Json> {
        if self.levels() < 2 {
            return Vec::new();
        }
        self.groups
            .iter()
            .flatten()
            .filter(|group| group.children.is_none() && self.is_open(&[group.id()]))
            .map(|group| group.key.clone())
            .collect()
    }

    /// The expanded paths, as JSON key lists (for the view of point 59).
    pub fn expanded(&self) -> Vec<Vec<Json>> {
        self.expanded
            .iter()
            .map(|path| {
                path.iter()
                    .map(|id| serde_json::from_str(id).unwrap_or(Json::Null))
                    .collect()
            })
            .collect()
    }

    /// Replaces the expanded set (restoring a view).
    pub fn set_expanded(&mut self, paths: &[Vec<Json>]) {
        self.expanded = paths
            .iter()
            .filter(|path| !path.is_empty() && path.len() <= self.levels())
            .map(|path| path.iter().map(Json::to_string).collect())
            .collect();
        self.relayout();
    }

    /// Opens or closes a group. Answers whether it is open now.
    pub fn toggle(&mut self, keys: &[Json]) -> bool {
        let path: Path = keys.iter().map(Json::to_string).collect();
        let open = if self.expanded.remove(&path) {
            false
        } else {
            self.expanded.insert(path);
            true
        };
        self.relayout();
        open
    }

    fn is_open(&self, path: &[String]) -> bool {
        self.expanded.contains(path)
    }

    /// Rows a level-1 group occupies in the display list, its header included.
    fn size_of(&self, group: &Group) -> u64 {
        let path = [group.id()];
        if !self.is_open(&path) {
            return 1;
        }
        if self.levels() < 2 {
            return 1 + group.count;
        }
        // Not fetched yet: the header alone, until the level-2 query answers.
        let Some(children) = &group.children else {
            return 1;
        };
        1 + children
            .iter()
            .map(|child| self.child_size(&group.id(), child))
            .sum::<u64>()
    }

    fn child_size(&self, parent: &str, child: &Group) -> u64 {
        if self.is_open(&[parent.to_owned(), child.id()]) {
            1 + child.count
        } else {
            1
        }
    }

    fn relayout(&mut self) {
        let mut starts = Vec::new();
        let mut at = 0u64;
        for group in self.groups.iter().flatten() {
            starts.push(at);
            at += self.size_of(group);
        }
        starts.push(at);
        self.starts = starts;
    }

    /// Length of the display list: group headers plus the rows of open groups.
    ///
    /// This is what `aria-rowcount` is computed from — the one truth.
    pub fn len(&self) -> u64 {
        match self.starts.last() {
            // The total row, after the last group — and only once there are
            // groups: a total over nothing would announce a row of zeros.
            Some(&end) if end > 0 => end + 1,
            _ => 0,
        }
    }

    /// End of the groups, before the total row.
    fn groups_end(&self) -> u64 {
        self.starts.last().copied().unwrap_or(0)
    }

    /// Whether the display list is empty (no groups, or not loaded).
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Data rows across every group — what the status line calls "matches".
    ///
    /// Not [`len`](Self::len): a reader told "57 matches" for 52 rows plus five
    /// group headers would be told something false.
    pub fn row_count(&self) -> u64 {
        self.groups.iter().flatten().map(|group| group.count).sum()
    }

    /// What sits at `position`, or `None` past the end.
    ///
    /// Binary search over the level-1 starts, then a walk inside one group — so
    /// the cost is logarithmic in the number of groups, not linear in rows.
    pub fn item_at(&self, position: u64) -> Option<Item> {
        let groups = self.groups.as_ref()?;
        if position >= self.len() {
            return None;
        }
        if position == self.groups_end() {
            return Some(Item::Total {
                count: self.row_count(),
                aggregates: self.total.clone().unwrap_or_default(),
            });
        }
        // The last start that is <= position.
        let index = match self.starts.binary_search(&position) {
            Ok(exact) => exact,
            Err(after) => after - 1,
        };
        let group = groups.get(index)?;
        let inside = position - self.starts[index];
        let path = [group.id()];

        if inside == 0 {
            return Some(Item::Group {
                level: 1,
                keys: vec![group.key.clone()],
                value: group.value.clone(),
                count: group.count,
                expanded: self.is_open(&path),
                aggregates: group.aggregates.clone(),
            });
        }
        let mut rest = inside - 1;

        if self.levels() < 2 {
            return Some(Item::Row {
                keys: vec![group.key.clone()],
                offset: rest,
            });
        }

        for child in group.children.iter().flatten() {
            let size = self.child_size(&group.id(), child);
            if rest < size {
                let keys = vec![group.key.clone(), child.key.clone()];
                return Some(if rest == 0 {
                    Item::Group {
                        level: 2,
                        keys,
                        value: child.value.clone(),
                        count: child.count,
                        expanded: size > 1,
                        aggregates: child.aggregates.clone(),
                    }
                } else {
                    Item::Row {
                        keys,
                        offset: rest - 1,
                    }
                });
            }
            rest -= size;
        }
        None
    }

    /// Where the header of the group with these keys sits, if it is visible.
    ///
    /// Test-only: it is how the tests **prove** what the element relies on
    /// without asking — a group's own position does not move when it opens
    /// (only what comes after it does), so the focus can simply stay where it
    /// is; a level-2 group, though, loses its place when its parent closes.
    #[cfg(test)]
    pub fn position_of(&self, keys: &[Json]) -> Option<u64> {
        let groups = self.groups.as_ref()?;
        let first = keys.first()?;
        let index = groups.iter().position(|group| &group.key == first)?;
        let start = self.starts[index];
        let Some(second) = keys.get(1) else {
            return Some(start);
        };
        let group = &groups[index];
        if !self.is_open(&[group.id()]) {
            return None;
        }
        let mut at = start + 1;
        for child in group.children.iter().flatten() {
            if &child.key == second {
                return Some(at);
            }
            at += self.child_size(&group.id(), child);
        }
        None
    }

    /// The row slices a window of `count` positions from `start` needs.
    ///
    /// Consecutive rows of the same group become **one** fetch — that is the
    /// "1 + K queries" of the prototype: one per group the window touches, not
    /// one per row.
    pub fn fetches(&self, start: u64, count: u64) -> Vec<Fetch> {
        let mut out: Vec<Fetch> = Vec::new();
        let end = start.saturating_add(count).min(self.len());
        for position in start..end {
            let Some(Item::Row { keys, offset }) = self.item_at(position) else {
                continue;
            };
            if let Some(last) = out.last_mut()
                && last.keys == keys
                && last.offset + last.limit == offset
                && last.position + last.limit == position
            {
                last.limit += 1;
                continue;
            }
            out.push(Fetch {
                keys,
                offset,
                limit: 1,
                position,
            });
        }
        out
    }

    /// The filter that selects one group's rows: the page's filter **and** each
    /// key on the path.
    ///
    /// NULL is `is_null`, not `eq null` — under three-valued logic the latter is
    /// *unknown* and matches nothing (S1), which would show an open NULL group
    /// with no rows in it.
    pub fn filter_for(&self, keys: &[Json], base: Option<&FilterExpr>) -> Option<FilterExpr> {
        let mut parts: Vec<FilterExpr> = base.cloned().into_iter().collect();
        for (name, key) in self.by.iter().zip(keys) {
            let Ok(field) = FieldName::new(name) else {
                continue;
            };
            parts.push(if key.is_null() {
                FilterExpr::IsNull { field }
            } else {
                FilterExpr::Cmp {
                    field,
                    op: CmpOp::Eq,
                    value: key.clone(),
                }
            });
        }
        match parts.len() {
            0 => None,
            1 => parts.pop(),
            _ => Some(FilterExpr::And(parts)),
        }
    }
}

/// The alias the *i*-th aggregate is asked under.
///
/// By position, not by column name: `__og_a` plus a 63-character column name
/// would break the identifier limit, and the position is what the result is
/// read back by anyway.
pub fn aggregate_alias(index: usize) -> String {
    format!("__og_a{index}")
}

fn aggregate_list(aggregates: &[(String, Summary)]) -> Vec<Json> {
    let mut list = vec![serde_json::json!({ "fn": "count", "as": COUNT_ALIAS })];
    let functions = aggregates.iter().flat_map(|(column, summary)| {
        summary
            .functions()
            .iter()
            .map(move |function| (column, function))
    });
    for (index, (column, function)) in functions.enumerate() {
        list.push(serde_json::json!({
            "fn": function.as_str(),
            "field": column,
            "as": aggregate_alias(index),
        }));
    }
    list
}

/// How many aggregate columns a query asks for: a range is two.
fn aggregate_width(aggregates: &[(String, Summary)]) -> usize {
    aggregates
        .iter()
        .map(|(_, summary)| summary.functions().len())
        .sum()
}

/// A group's aggregate values, one slice per chosen summary — a range answers
/// two values (its `min` and `max`), everything else one. Missing values are
/// NULL, so a short answer shows empty cells rather than shifted ones.
pub fn split<'a>(aggregates: &[(String, Summary)], values: &'a [Value]) -> Vec<&'a [Value]> {
    let mut out = Vec::with_capacity(aggregates.len());
    let mut at = 0;
    for (_, summary) in aggregates {
        let width = summary.functions().len();
        let end = (at + width).min(values.len());
        out.push(&values[at.min(end)..end]);
        at += width;
    }
    out
}

/// The group query of one level: every distinct key of `column` with its row
/// count and the chosen aggregates, **NULL last** (S3's default, stated
/// explicitly the way compilers are told to).
///
/// No `limit`: the groups are what the display list is computed from, and a
/// display list computed from some of them would lie about `aria-rowcount`.
pub fn group_query_json(
    source: &str,
    column: &str,
    filter: Option<&FilterExpr>,
    aggregates: &[(String, Summary)],
) -> String {
    let mut select = vec![
        Json::String(column.to_owned()),
        Json::String(COUNT_ALIAS.to_owned()),
    ];
    select
        .extend((0..aggregate_width(aggregates)).map(|index| Json::String(aggregate_alias(index))));
    let mut query = serde_json::json!({
        "source": source,
        "group": [column],
        "aggregate": aggregate_list(aggregates),
        "select": select,
        "sort": [{ "field": column, "direction": "asc", "nulls": "last" }],
    });
    if let Some(filter) = filter {
        query["filter"] = serde_json::to_value(filter).expect("a filter expression serializes");
    }
    query.to_string()
}

/// The grand total: the same aggregates without `group` — exactly one row,
/// even over an empty result (S11).
pub fn total_query_json(
    source: &str,
    filter: Option<&FilterExpr>,
    aggregates: &[(String, Summary)],
) -> String {
    let mut select = vec![Json::String(COUNT_ALIAS.to_owned())];
    select
        .extend((0..aggregate_width(aggregates)).map(|index| Json::String(aggregate_alias(index))));
    let mut query = serde_json::json!({
        "source": source,
        "aggregate": aggregate_list(aggregates),
        "select": select,
    });
    if let Some(filter) = filter {
        query["filter"] = serde_json::to_value(filter).expect("a filter expression serializes");
    }
    query.to_string()
}

/// Reads a total query's result: the aggregates, in order, after the count.
pub fn total_from(result: &opengrid_datasource::QueryResult) -> Vec<Value> {
    result
        .columns
        .iter()
        .skip(1)
        .map(|column| column.first().cloned().unwrap_or(Value::Null))
        .collect()
}

/// Reads a group query's result into groups, in the order it came.
pub fn groups_from(result: &opengrid_datasource::QueryResult) -> Result<Vec<Group>, String> {
    let keys = result
        .columns
        .first()
        .ok_or_else(|| "the group query answered no key column".to_owned())?;
    let counts = result
        .columns
        .get(1)
        .ok_or_else(|| "the group query answered no count".to_owned())?;
    Ok(keys
        .iter()
        .zip(counts)
        .enumerate()
        .map(|(row, (key, count))| {
            let count = match count {
                Value::Int64(count) => (*count).max(0) as u64,
                _ => 0,
            };
            let mut group = Group::new(key.clone(), count);
            group.aggregates = result
                .columns
                .iter()
                .skip(2)
                .map(|column| column.get(row).cloned().unwrap_or(Value::Null))
                .collect();
            group
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(key: &str) -> Value {
        Value::Utf8(key.to_owned())
    }

    /// DE 3, FR 1, (empty) 2, NULL 2 — a small grid with both kinds of "no
    /// value", which are different groups.
    fn one_level() -> Grouping {
        let mut grouping = Grouping::new(vec!["country".to_owned()]);
        grouping.set_groups(vec![
            Group::new(text("DE"), 3),
            Group::new(text("FR"), 1),
            Group::new(text(""), 2),
            Group::new(Value::Null, 2),
        ]);
        grouping
    }

    fn key(value: &str) -> Json {
        Json::String(value.to_owned())
    }

    #[test]
    fn all_closed_is_one_row_per_group_and_the_total() {
        let grouping = one_level();
        // Four headers and the total row after them.
        assert_eq!(grouping.len(), 5);
        assert_eq!(grouping.row_count(), 8);
        for position in 0..4 {
            assert!(matches!(
                grouping.item_at(position),
                Some(Item::Group {
                    level: 1,
                    expanded: false,
                    ..
                })
            ));
        }
        assert!(matches!(
            grouping.item_at(4),
            Some(Item::Total { count: 8, .. })
        ));
        assert_eq!(grouping.item_at(5), None);
    }

    #[test]
    fn opening_a_group_inserts_its_rows_after_it_and_nothing_before() {
        let mut grouping = one_level();
        assert!(grouping.toggle(&[key("FR")]));
        // DE, FR, FR row 0, (empty), NULL, total.
        assert_eq!(grouping.len(), 6);
        assert!(matches!(grouping.item_at(0), Some(Item::Group { .. })));
        assert!(matches!(
            grouping.item_at(1),
            Some(Item::Group {
                expanded: true,
                count: 1,
                ..
            })
        ));
        assert_eq!(
            grouping.item_at(2),
            Some(Item::Row {
                keys: vec![key("FR")],
                offset: 0
            })
        );
        assert!(matches!(grouping.item_at(3), Some(Item::Group { .. })));
        // The toggled group did not move, which is what keeps the focus on it.
        assert_eq!(grouping.position_of(&[key("FR")]), Some(1));
    }

    #[test]
    fn all_open_is_every_row_plus_every_header() {
        let mut grouping = one_level();
        for group in ["DE", "FR", ""] {
            grouping.toggle(&[key(group)]);
        }
        grouping.toggle(&[Json::Null]);
        assert_eq!(grouping.len(), 8 + 4 + 1);
        // The last position is the last row of the NULL group.
        assert_eq!(
            grouping.item_at(11),
            Some(Item::Row {
                keys: vec![Json::Null],
                offset: 1
            })
        );
        assert!(matches!(grouping.item_at(12), Some(Item::Total { .. })));
        assert_eq!(grouping.item_at(13), None);
    }

    #[test]
    fn closing_takes_the_rows_out_again() {
        let mut grouping = one_level();
        grouping.toggle(&[key("DE")]);
        assert_eq!(grouping.len(), 8);
        assert!(!grouping.toggle(&[key("DE")]));
        assert_eq!(grouping.len(), 5);
    }

    #[test]
    fn null_and_empty_are_two_groups_with_two_filters() {
        // S10 and S14 — and S1: `eq null` would match nothing, so an open NULL
        // group would show no rows at all.
        let grouping = one_level();
        assert_eq!(
            grouping.filter_for(&[Json::Null], None),
            Some(FilterExpr::IsNull {
                field: FieldName::new("country").unwrap()
            })
        );
        assert_eq!(
            grouping.filter_for(&[key("")], None),
            Some(FilterExpr::Cmp {
                field: FieldName::new("country").unwrap(),
                op: CmpOp::Eq,
                value: key(""),
            })
        );
    }

    #[test]
    fn the_page_filter_is_kept_under_the_group_filter() {
        let grouping = one_level();
        let base = FilterExpr::IsNotNull {
            field: FieldName::new("amount").unwrap(),
        };
        let Some(FilterExpr::And(parts)) = grouping.filter_for(&[key("DE")], Some(&base)) else {
            panic!("the group filter must be combined with the page's");
        };
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0], base);
    }

    #[test]
    fn a_window_asks_one_fetch_per_group_it_touches() {
        let mut grouping = one_level();
        grouping.toggle(&[key("DE")]);
        grouping.toggle(&[key("")]);
        // DE, DE0, DE1, DE2, FR, (empty), E0, E1, NULL
        let fetches = grouping.fetches(0, 9);
        assert_eq!(
            fetches,
            vec![
                Fetch {
                    keys: vec![key("DE")],
                    offset: 0,
                    limit: 3,
                    position: 1
                },
                Fetch {
                    keys: vec![key("")],
                    offset: 0,
                    limit: 2,
                    position: 6
                },
            ]
        );
    }

    #[test]
    fn a_window_that_starts_inside_a_group_fetches_from_the_middle() {
        let mut grouping = one_level();
        grouping.toggle(&[key("DE")]);
        let fetches = grouping.fetches(2, 2);
        assert_eq!(
            fetches,
            vec![Fetch {
                keys: vec![key("DE")],
                offset: 1,
                limit: 2,
                position: 2
            }]
        );
    }

    #[test]
    fn a_group_larger_than_the_window_is_fetched_in_slices() {
        let mut grouping = Grouping::new(vec!["country".to_owned()]);
        grouping.set_groups(vec![Group::new(text("DE"), 100_000)]);
        grouping.toggle(&[key("DE")]);
        assert_eq!(grouping.len(), 100_002);
        let fetches = grouping.fetches(50_000, 40);
        assert_eq!(fetches.len(), 1);
        assert_eq!(fetches[0].offset, 49_999);
        assert_eq!(fetches[0].limit, 40);
    }

    #[test]
    fn the_last_window_ends_at_the_last_row() {
        let mut grouping = one_level();
        grouping.toggle(&[Json::Null]);
        let fetches = grouping.fetches(4, 40);
        assert_eq!(fetches.len(), 1);
        assert_eq!(fetches[0].limit, 2);
    }

    #[test]
    fn a_group_with_one_row() {
        let mut grouping = one_level();
        grouping.toggle(&[key("FR")]);
        assert_eq!(grouping.fetches(0, 10).len(), 1);
        assert_eq!(grouping.fetches(0, 10)[0].limit, 1);
    }

    #[test]
    fn no_groups_is_an_empty_list() {
        let mut grouping = Grouping::new(vec!["country".to_owned()]);
        assert!(grouping.is_empty());
        grouping.set_groups(Vec::new());
        assert!(grouping.is_empty());
        assert_eq!(grouping.item_at(0), None);
        assert!(grouping.fetches(0, 10).is_empty());
    }

    fn two_level() -> Grouping {
        let mut grouping = Grouping::new(vec!["country".to_owned(), "customer".to_owned()]);
        grouping.set_groups(vec![Group::new(text("DE"), 5), Group::new(text("FR"), 2)]);
        grouping
    }

    #[test]
    fn a_second_level_is_empty_until_it_is_fetched() {
        let mut grouping = two_level();
        grouping.toggle(&[key("DE")]);
        // Open, but its subgroups have not been asked for yet: header only
        // (DE, FR, total).
        assert_eq!(grouping.len(), 3);
        assert_eq!(grouping.missing_children(), vec![key("DE")]);

        grouping.set_children(
            &key("DE"),
            vec![Group::new(text("Alpha"), 3), Group::new(Value::Null, 2)],
        );
        assert!(grouping.missing_children().is_empty());
        // DE, Alpha, (NULL), FR, total
        assert_eq!(grouping.len(), 5);
        assert!(matches!(
            grouping.item_at(1),
            Some(Item::Group {
                level: 2,
                expanded: false,
                count: 3,
                ..
            })
        ));
    }

    #[test]
    fn opening_a_second_level_group_puts_its_rows_under_it() {
        let mut grouping = two_level();
        grouping.toggle(&[key("DE")]);
        grouping.set_children(
            &key("DE"),
            vec![Group::new(text("Alpha"), 3), Group::new(Value::Null, 2)],
        );
        grouping.toggle(&[key("DE"), Json::Null]);
        // DE, Alpha, NULL, NULL0, NULL1, FR, total
        assert_eq!(grouping.len(), 7);
        assert_eq!(
            grouping.item_at(4),
            Some(Item::Row {
                keys: vec![key("DE"), Json::Null],
                offset: 1
            })
        );
        assert_eq!(grouping.position_of(&[key("DE"), Json::Null]), Some(2));

        // Closing the parent hides the child, which then has no position.
        grouping.toggle(&[key("DE")]);
        assert_eq!(grouping.len(), 3);
        assert_eq!(grouping.position_of(&[key("DE"), Json::Null]), None);
    }

    #[test]
    fn expanded_survives_a_reload_of_the_groups() {
        // A filter that hides a group and one that brings it back should find it
        // open again.
        let mut grouping = one_level();
        grouping.toggle(&[key("DE")]);
        grouping.invalidate();
        assert!(grouping.is_empty());
        grouping.set_groups(vec![Group::new(text("DE"), 3)]);
        // DE, its three rows, total.
        assert_eq!(grouping.len(), 5);
    }

    #[test]
    fn expanded_round_trips_through_json() {
        let mut grouping = two_level();
        grouping.toggle(&[key("DE"), Json::Null]);
        let saved = grouping.expanded();
        let mut other = two_level();
        other.set_expanded(&saved);
        assert_eq!(other.expanded(), saved);
    }

    #[test]
    fn group_by_takes_at_most_two_and_refuses_a_third() {
        assert_eq!(
            parse_group_by(Some(" country , customer ")),
            Ok(vec!["country".to_owned(), "customer".to_owned()])
        );
        assert_eq!(parse_group_by(Some("a,a")), Ok(vec!["a".to_owned()]));
        assert_eq!(parse_group_by(None), Ok(Vec::new()));
        assert_eq!(parse_group_by(Some("a,b,c")), Err("a,b,c".to_owned()));
    }

    #[test]
    fn only_repeating_types_are_groupable() {
        assert!(groupable(DataType::Utf8));
        assert!(groupable(DataType::Date));
        assert!(!groupable(DataType::Float64));
        assert!(!groupable(DataType::Timestamp));
    }

    /// The total row is never asked for as rows: it is one aggregate query, and
    /// a window that reaches it fetches nothing for it.
    #[test]
    fn a_window_over_the_total_fetches_no_rows_for_it() {
        let grouping = one_level();
        assert!(grouping.fetches(0, 10).is_empty());
    }

    /// No groups, no total: a row of zeros over nothing would be announced as
    /// if it said something.
    #[test]
    fn there_is_no_total_without_groups() {
        let mut grouping = Grouping::new(vec!["country".to_owned()]);
        grouping.set_groups(Vec::new());
        assert_eq!(grouping.len(), 0);
    }

    /// A different choice of aggregates makes the loaded counts stale: they
    /// were asked together with the old ones.
    #[test]
    fn choosing_other_aggregates_reloads_the_groups() {
        let mut grouping = one_level();
        grouping.set_aggregates(vec![(
            "amount".to_owned(),
            Summary::Fn(opengrid_query::AggregateFn::Sum),
        )]);
        assert!(!grouping.is_loaded());
        grouping.set_groups(vec![Group::new(text("DE"), 3)]);
        grouping.set_aggregates(vec![(
            "amount".to_owned(),
            Summary::Fn(opengrid_query::AggregateFn::Sum),
        )]);
        assert!(grouping.is_loaded(), "the same choice changes nothing");
    }

    /// A range is asked as `min` and `max` of its column (F7), and the answer
    /// is split back so that every later aggregate keeps its own value.
    #[test]
    fn a_range_is_asked_as_min_and_max_and_split_back() {
        let aggregates = vec![
            ("ordered_on".to_owned(), Summary::Range),
            (
                "amount".to_owned(),
                Summary::Fn(opengrid_query::AggregateFn::Sum),
            ),
        ];
        let grouped: Json =
            serde_json::from_str(&group_query_json("orders", "country", None, &aggregates))
                .unwrap();
        assert_eq!(
            grouped["select"],
            serde_json::json!(["country", COUNT_ALIAS, "__og_a0", "__og_a1", "__og_a2"])
        );
        let asked: Vec<(&str, &str)> = grouped["aggregate"]
            .as_array()
            .unwrap()
            .iter()
            .skip(1)
            .map(|entry| {
                (
                    entry["fn"].as_str().unwrap(),
                    entry["field"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            asked,
            [
                ("min", "ordered_on"),
                ("max", "ordered_on"),
                ("sum", "amount")
            ]
        );

        let values = [Value::Int64(1), Value::Int64(9), Value::Int64(42)];
        let split = split(&aggregates, &values);
        assert_eq!(split, [&values[..2], &values[2..]]);
        // A short answer leaves the missing ones empty instead of shifting them.
        assert_eq!(split_short(&aggregates), [0, 0]);
    }

    fn split_short(aggregates: &[(String, Summary)]) -> Vec<usize> {
        split(aggregates, &[])
            .iter()
            .map(|values| values.len())
            .collect()
    }

    /// The aggregates travel by position, after the key and the count.
    #[test]
    fn the_queries_ask_the_aggregates_by_position() {
        let aggregates = vec![
            (
                "amount".to_owned(),
                Summary::Fn(opengrid_query::AggregateFn::Sum),
            ),
            (
                "qty".to_owned(),
                Summary::Fn(opengrid_query::AggregateFn::Avg),
            ),
        ];
        let grouped: Json =
            serde_json::from_str(&group_query_json("orders", "country", None, &aggregates))
                .unwrap();
        assert_eq!(
            grouped["select"],
            serde_json::json!(["country", COUNT_ALIAS, "__og_a0", "__og_a1"])
        );
        assert_eq!(grouped["aggregate"][2]["field"], "qty");
        assert_eq!(grouped["aggregate"][2]["fn"], "avg");

        let total: Json =
            serde_json::from_str(&total_query_json("orders", None, &aggregates)).unwrap();
        assert!(
            total.get("group").is_none(),
            "the total is one row, not a group"
        );
        assert_eq!(
            total["select"],
            serde_json::json!([COUNT_ALIAS, "__og_a0", "__og_a1"])
        );
    }

    #[test]
    fn the_group_query_puts_null_last_and_has_no_limit() {
        let query: Json =
            serde_json::from_str(&group_query_json("orders", "country", None, &[])).unwrap();
        assert_eq!(query["sort"][0]["nulls"], "last");
        assert!(query.get("limit").is_none());
        assert_eq!(query["aggregate"][0]["as"], COUNT_ALIAS);
    }
}
