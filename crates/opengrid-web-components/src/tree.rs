//! The tree mode of the grid (plan point 123, E38): the display list of a
//! tree, as arithmetic over the levels that are loaded.
//!
//! A level — the children of one node, or the roots — is loaded whole the
//! first time it is opened, with each node's child count up front (T4). From
//! the loaded levels and the set of open nodes the display list follows: a
//! node, then (when it is open) its children's subtrees, in the order the
//! source sorted them (T6). So, as with grouping, there is one truth about
//! `aria-rowcount`, and every row also knows its depth and its place among its
//! siblings — what `aria-level`, `aria-posinset` and `aria-setsize` say.
//!
//! A node is named by its key's canonical JSON (keys are unique, T1), which is
//! what the view stores and what `under` is asked with.

use std::collections::{BTreeSet, HashMap};

use opengrid_datasource::QueryResult;
use opengrid_json::Json;
use opengrid_types::Value;

/// The most siblings a level may have: a level is loaded whole, and a
/// larger one is a filter's job, said in a sentence (issue #135). The
/// query limit's maximum: the level's `total_count` tells a larger one.
pub const MAX_LEVEL: u64 = 10_000;

/// One loaded level: its rows, and what each of them is.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Level {
    /// The values of each row, in the columns the grid shows.
    pub rows: Vec<Vec<Value>>,
    /// Each node's key, as the wire writes it.
    pub keys: Vec<Json>,
    /// How many visible children each node has (T4).
    pub children: Vec<u64>,
    /// Whether each node matches, or is context (T5).
    pub matched: Vec<bool>,
    /// Each node's subtree aggregates (T7), in the order they were asked —
    /// empty when none were.
    pub aggregates: Vec<Vec<Value>>,
}

impl Level {
    /// Reads a level from a tree query's answer: `width` shown columns, then
    /// the key at `key_column` (which may be one of them).
    pub fn from_result(
        result: &QueryResult,
        width: usize,
        key_column: usize,
    ) -> Result<Self, String> {
        let tree = result
            .tree
            .as_ref()
            .ok_or("the answer to a tree query carries no tree part")?;
        let rows = result.row_count();
        let column = |index: usize| -> Result<&Vec<Value>, String> {
            result
                .columns
                .get(index)
                .ok_or_else(|| "the answer has fewer columns than asked".to_owned())
        };
        let keys = column(key_column)?
            .iter()
            .map(opengrid_json::ToJson::to_json)
            .collect();
        let mut values = vec![Vec::with_capacity(width); rows];
        for index in 0..width {
            for (row, value) in column(index)?.iter().enumerate() {
                values[row].push(value.clone());
            }
        }
        // Column by column on the wire; row by row here, like the values.
        let mut aggregates = vec![Vec::with_capacity(tree.aggregates.len()); rows];
        for column in &tree.aggregates {
            for (row, value) in column.iter().enumerate().take(rows) {
                aggregates[row].push(value.clone());
            }
        }
        Ok(Self {
            rows: values,
            keys,
            children: tree.children.clone(),
            matched: tree.matched.clone(),
            aggregates,
        })
    }
}

/// One position of the display list.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// The level the node belongs to (`None`: the roots), and its index there.
    pub parent: Option<String>,
    pub index: usize,
    /// The node's identity: its key's canonical JSON.
    pub id: String,
    /// 1 for a root.
    pub depth: u16,
    /// Its place among its siblings, 1-based, and how many they are.
    pub position: u64,
    pub siblings: u64,
    /// Its visible children (T4) — 0 is a leaf.
    pub children: u64,
    pub expanded: bool,
    /// Open, and its children not loaded yet: the row is busy.
    pub loading: bool,
    /// A match, or context (T5).
    pub matched: bool,
}

/// The tree state of one grid.
#[derive(Clone, Debug, Default)]
pub struct Tree {
    key: String,
    parent: String,
    /// Loaded levels, by the id of the node they belong to (`None`: roots).
    levels: HashMap<Option<String>, Level>,
    expanded: BTreeSet<String>,
    /// The selected nodes, by id (issue #135): a key names a record, so the
    /// selection survives a sort, a filter and a node closing over it.
    selected: BTreeSet<String>,
    /// Where a range selection starts: the node of the last plain toggle.
    anchor: Option<String>,
    flat: Vec<Entry>,
    matches: u64,
    orphans: u64,
    /// The summaries each node shows over its subtree (T7), by column.
    aggregates: Vec<(String, crate::presentation::Summary)>,
}

impl Tree {
    pub fn new(key: impl Into<String>, parent: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            parent: parent.into(),
            ..Self::default()
        }
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn parent(&self) -> &str {
        &self.parent
    }

    /// Whether the roots are loaded.
    pub fn is_loaded(&self) -> bool {
        self.levels.contains_key(&None)
    }

    /// Drops every loaded level (a filter or a sort changed), keeping what is
    /// open: the reader finds it open again.
    pub fn invalidate(&mut self) {
        self.levels.clear();
        self.flat.clear();
    }

    /// Takes a loaded level, and — for the roots — the tree's own counts.
    pub fn set_level(&mut self, under: Option<String>, level: Level, matches: u64, orphans: u64) {
        if under.is_none() {
            self.matches = matches;
            self.orphans = orphans;
        }
        self.levels.insert(under, level);
        self.relayout();
    }

    /// Matches in the whole tree (T5) — what the status line counts.
    pub fn matches(&self) -> u64 {
        self.matches
    }

    /// Nodes without an existing parent, shown as roots (T2).
    pub fn orphans(&self) -> u64 {
        self.orphans
    }

    /// The open nodes, each as `[key]` — the view's `expanded`, in the shape
    /// a group's path has.
    pub fn expanded(&self) -> Vec<Vec<Json>> {
        self.expanded
            .iter()
            .map(|id| vec![opengrid_json::from_str(id).unwrap_or(Json::Null)])
            .collect()
    }

    /// Replaces the open nodes (restoring a view): each `[key]`. A key that is
    /// no node opens nothing; a longer path is a group's, not a node's.
    pub fn set_expanded(&mut self, paths: &[Vec<Json>]) {
        self.expanded = paths
            .iter()
            .filter(|path| path.len() == 1)
            .map(|path| path[0].to_string())
            .collect();
        self.relayout();
    }

    /// Opens or closes the node at `position`. Answers whether it is open now,
    /// or `None` for a leaf or a position past the end.
    pub fn toggle(&mut self, position: u64) -> Option<bool> {
        let entry = self.entry_at(position)?.clone();
        if entry.children == 0 {
            return None;
        }
        let open = if self.expanded.remove(&entry.id) {
            false
        } else {
            self.expanded.insert(entry.id.clone());
            true
        };
        self.relayout();
        Some(open)
    }

    /// The open nodes that are shown and whose children are not loaded yet —
    /// what has to be asked before the display list is whole.
    pub fn missing(&self) -> Vec<Json> {
        self.flat
            .iter()
            .filter(|entry| entry.loading)
            .map(|entry| self.key_of(entry))
            .collect()
    }

    fn key_of(&self, entry: &Entry) -> Json {
        self.levels[&entry.parent].keys[entry.index].clone()
    }

    fn relayout(&mut self) {
        let mut flat = Vec::new();
        self.walk(None, 1, &mut flat);
        self.flat = flat;
    }

    fn walk(&self, under: Option<String>, depth: u16, flat: &mut Vec<Entry>) {
        let Some(level) = self.levels.get(&under) else {
            return;
        };
        let siblings = level.keys.len() as u64;
        for index in 0..level.keys.len() {
            let id = level.keys[index].to_string();
            let children = level.children[index];
            let expanded = children > 0 && self.expanded.contains(&id);
            let loaded = self.levels.contains_key(&Some(id.clone()));
            flat.push(Entry {
                parent: under.clone(),
                index,
                id: id.clone(),
                depth,
                position: index as u64 + 1,
                siblings,
                children,
                expanded,
                loading: expanded && !loaded,
                matched: level.matched[index],
            });
            if expanded && loaded {
                self.walk(Some(id), depth + 1, flat);
            }
        }
    }

    /// Selects or deselects the node at `position`, and makes it the anchor.
    /// Answers whether anything changed.
    pub fn toggle_selected(&mut self, position: u64) -> bool {
        let Some(id) = self.entry_at(position).map(|entry| entry.id.clone()) else {
            return false;
        };
        if !self.selected.remove(&id) {
            self.selected.insert(id.clone());
        }
        self.anchor = Some(id);
        true
    }

    /// Selects the shown nodes from the anchor to `position`, keeping what is
    /// selected. Without an anchor in view, a plain toggle.
    pub fn extend_selected(&mut self, position: u64) -> bool {
        let anchor = self
            .anchor
            .as_ref()
            .and_then(|anchor| self.flat.iter().position(|entry| entry.id == *anchor));
        let Some(anchor) = anchor else {
            return self.toggle_selected(position);
        };
        let Ok(position) = usize::try_from(position) else {
            return false;
        };
        let (low, high) = (anchor.min(position), anchor.max(position));
        let before = self.selected.len();
        for entry in self.flat.iter().skip(low).take(high + 1 - low) {
            self.selected.insert(entry.id.clone());
        }
        self.selected.len() != before
    }

    /// Selects every node shown — "select all" in a tree, whose closed levels
    /// were never loaded.
    pub fn select_shown(&mut self) -> bool {
        let before = self.selected.len();
        self.selected
            .extend(self.flat.iter().map(|entry| entry.id.clone()));
        self.selected.len() != before
    }

    /// Whether every node shown is selected (and some are shown).
    pub fn all_shown_selected(&self) -> bool {
        !self.flat.is_empty()
            && self
                .flat
                .iter()
                .all(|entry| self.selected.contains(&entry.id))
    }

    /// Drops the whole selection, shown or not.
    pub fn clear_selected(&mut self) -> bool {
        self.anchor = None;
        let changed = !self.selected.is_empty();
        self.selected.clear();
        changed
    }

    /// The display positions of the selected nodes that are shown, ascending.
    pub fn selected_positions(&self) -> Vec<u64> {
        (0..self.len())
            .filter(|position| self.selected.contains(&self.flat[*position as usize].id))
            .collect()
    }

    /// The keys of the selected nodes: the shown ones in display order, then
    /// those under a closed node or outside the filter.
    pub fn selected_keys(&self) -> Vec<Json> {
        let shown: BTreeSet<&String> = self
            .flat
            .iter()
            .filter(|entry| self.selected.contains(&entry.id))
            .map(|entry| &entry.id)
            .collect();
        self.flat
            .iter()
            .filter(|entry| self.selected.contains(&entry.id))
            .map(|entry| entry.id.as_str())
            .chain(
                self.selected
                    .iter()
                    .filter(|id| !shown.contains(id))
                    .map(String::as_str),
            )
            .map(|id| opengrid_json::from_str(id).unwrap_or(Json::Null))
            .collect()
    }

    /// Length of the display list — what `aria-rowcount` is computed from.
    pub fn len(&self) -> u64 {
        self.flat.len() as u64
    }

    pub fn entry_at(&self, position: u64) -> Option<&Entry> {
        self.flat.get(usize::try_from(position).ok()?)
    }

    /// The position of the parent of the node at `position` (← on a closed
    /// node), or `None` for a root.
    pub fn parent_of(&self, position: u64) -> Option<u64> {
        let depth = self.entry_at(position)?.depth;
        (0..position)
            .rev()
            .find(|at| self.entry_at(*at).is_some_and(|entry| entry.depth < depth))
    }

    /// The position of the first child of the open node at `position` (→ on
    /// an open node), once its children are shown.
    pub fn first_child_of(&self, position: u64) -> Option<u64> {
        let entry = self.entry_at(position)?;
        let next = self.entry_at(position + 1)?;
        (entry.expanded && next.depth == entry.depth + 1).then_some(position + 1)
    }

    /// Sets the summaries the nodes show over their subtrees (T7). Others
    /// than before make every loaded level stale: they were asked with the
    /// old ones.
    pub fn set_aggregates(&mut self, aggregates: Vec<(String, crate::presentation::Summary)>) {
        if self.aggregates != aggregates {
            self.aggregates = aggregates;
            self.invalidate();
        }
    }

    /// The summaries the nodes show over their subtrees.
    pub fn aggregates(&self) -> &[(String, crate::presentation::Summary)] {
        &self.aggregates
    }

    /// The subtree aggregates of the node at `position`, in the order of
    /// [`aggregates`](Self::aggregates) — a range takes two.
    pub fn aggregate_values(&self, position: u64) -> Option<&[Value]> {
        let entry = self.flat.get(position as usize)?;
        self.levels[&entry.parent]
            .aggregates
            .get(entry.index)
            .map(Vec::as_slice)
    }

    /// The rows of `count` positions from `start`, column-major in `width`
    /// columns — the page the grid draws.
    pub fn page(&self, start: u64, count: u64, width: usize) -> Vec<Vec<Value>> {
        let end = start.saturating_add(count).min(self.len());
        let mut columns = vec![Vec::new(); width];
        for position in start..end {
            let entry = &self.flat[position as usize];
            let row = &self.levels[&entry.parent].rows[entry.index];
            for (column, values) in columns.iter_mut().enumerate() {
                values.push(row.get(column).cloned().unwrap_or(Value::Null));
            }
        }
        columns
    }
}

/// The JSON of a `tree` query part: one level, the children of `under` or
/// the roots.
pub fn tree_part(
    key: &str,
    parent: &str,
    under: Option<&Json>,
    aggregates: &[(String, crate::presentation::Summary)],
) -> Json {
    let mut part = opengrid_json::json!({
        "key": key,
        "parent": parent,
        "under": under.cloned().unwrap_or(Json::Null),
    });
    // T7: each node's subtree, with the summaries the columns show.
    if !aggregates.is_empty() {
        part["aggregate"] = Json::Array(crate::grouping::aggregate_functions(aggregates));
    }
    part
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level(keys: &[i64], children: &[u64]) -> Level {
        Level {
            rows: keys.iter().map(|key| vec![Value::Int64(*key)]).collect(),
            keys: keys.iter().map(|key| Json::from(*key)).collect(),
            children: children.to_vec(),
            matched: vec![true; keys.len()],
            aggregates: Vec::new(),
        }
    }

    /// The org chart of the conformance data: Sales(1) → North(2) →
    /// Alice(4) → Eve(10), Bob(5); South(3) → Carol(6); Partners(7) → Dave(8).
    fn chart() -> Tree {
        let mut tree = Tree::new("id", "parent_id");
        tree.set_level(None, level(&[1, 7, 9], &[2, 1, 0]), 10, 1);
        tree
    }

    fn summed() -> Vec<(String, crate::presentation::Summary)> {
        vec![(
            "revenue".to_owned(),
            crate::presentation::Summary::Fn(opengrid_query::AggregateFn::Sum),
        )]
    }

    /// Each node keeps its own subtree's aggregates (T7, #165), at whatever
    /// position it is shown.
    #[test]
    fn a_node_keeps_its_subtree_aggregates() {
        let mut tree = Tree::new("id", "parent_id");
        tree.set_aggregates(summed());
        let mut roots = level(&[1, 7, 9], &[2, 1, 0]);
        roots.aggregates = vec![
            vec![Value::Int64(223)],
            vec![Value::Int64(15)],
            vec![Value::Int64(7)],
        ];
        tree.set_level(None, roots, 10, 1);
        assert_eq!(tree.aggregate_values(1), Some(&[Value::Int64(15)][..]));
        assert_eq!(tree.aggregate_values(3), None, "no such position");
    }

    /// Other summaries make the loaded levels stale; the same ones do not.
    #[test]
    fn other_summaries_ask_the_levels_again() {
        let mut tree = chart();
        tree.set_aggregates(summed());
        assert!(!tree.is_loaded(), "asked with other summaries");
        tree.set_level(None, level(&[1, 7, 9], &[2, 1, 0]), 10, 1);
        tree.set_aggregates(summed());
        assert!(tree.is_loaded(), "the same summaries keep what is loaded");
    }

    #[test]
    fn a_closed_tree_is_its_roots() {
        let tree = chart();
        assert_eq!(tree.len(), 3);
        let root = tree.entry_at(0).unwrap();
        assert_eq!((root.depth, root.position, root.siblings), (1, 1, 3));
        assert!(!root.expanded && !root.loading);
        assert_eq!(
            tree.entry_at(2).unwrap().children,
            0,
            "the orphan is a leaf"
        );
    }

    /// Opening asks for the level, the row is busy until it arrives, and the
    /// display list then holds the children under their parent.
    #[test]
    fn opening_a_node_shows_its_children_under_it() {
        let mut tree = chart();
        assert_eq!(tree.toggle(0), Some(true));
        assert!(tree.entry_at(0).unwrap().loading);
        assert_eq!(tree.missing(), vec![Json::from(1)]);
        tree.set_level(Some("1".into()), level(&[2, 3], &[2, 1]), 0, 0);
        assert!(tree.missing().is_empty());
        assert_eq!(tree.len(), 5);
        let north = tree.entry_at(1).unwrap();
        assert_eq!((north.depth, north.position, north.siblings), (2, 1, 2));
        assert_eq!(
            tree.entry_at(3).unwrap().id,
            "7",
            "Partners after Sales' subtree"
        );
        // ← from North goes to Sales; → from Sales to North.
        assert_eq!(tree.parent_of(1), Some(0));
        assert_eq!(tree.first_child_of(0), Some(1));
        assert_eq!(tree.first_child_of(1), None, "North is closed");
        assert_eq!(tree.parent_of(0), None);
        // A leaf does not toggle.
        assert_eq!(tree.toggle(4), None);
        // Closing hides the subtree and keeps the level loaded.
        assert_eq!(tree.toggle(0), Some(false));
        assert_eq!(tree.len(), 3);
        assert_eq!(tree.toggle(0), Some(true));
        assert_eq!(tree.len(), 5, "no second query for a level already loaded");
    }

    /// The open set is what the view keeps; a reload finds the nodes open.
    #[test]
    fn what_is_open_survives_a_reload() {
        let mut tree = chart();
        tree.set_expanded(&[vec![Json::from(7)], vec![Json::from(99)], vec![]]);
        assert_eq!(
            tree.missing(),
            vec![Json::from(7)],
            "99 is no node: nothing opens"
        );
        tree.set_level(Some("7".into()), level(&[8], &[0]), 0, 0);
        assert_eq!(tree.len(), 4);
        tree.invalidate();
        assert!(!tree.is_loaded());
        tree.set_level(None, level(&[1, 7, 9], &[2, 1, 0]), 10, 1);
        assert_eq!(
            tree.missing(),
            vec![Json::from(7)],
            "asked again, still open"
        );
        assert_eq!(
            tree.expanded(),
            vec![vec![Json::from(7)], vec![Json::from(99)]]
        );
    }

    /// The selection is the nodes' keys: it stays when they close over it,
    /// and a range runs over what is shown.
    #[test]
    fn the_selection_names_nodes() {
        let mut tree = chart();
        tree.toggle(0);
        tree.set_level(Some("1".into()), level(&[2, 3], &[2, 1]), 0, 0);
        // Shown: 1, 2, 3, 7, 9 — select North, then extend to Partners.
        assert!(tree.toggle_selected(1));
        assert!(tree.extend_selected(3));
        assert_eq!(tree.selected_positions(), vec![1, 2, 3]);
        // Closing Sales hides North and South; they stay selected.
        tree.toggle(0);
        assert_eq!(tree.selected_positions(), vec![1]);
        assert_eq!(
            tree.selected_keys(),
            vec![Json::from(7), Json::from(2), Json::from(3)],
            "the shown first, then the hidden"
        );
        assert!(!tree.all_shown_selected());
        assert!(tree.select_shown());
        assert!(tree.all_shown_selected());
        assert!(tree.clear_selected());
        assert!(tree.selected_keys().is_empty());
    }

    #[test]
    fn a_page_is_the_rows_in_display_order() {
        let mut tree = chart();
        tree.toggle(0);
        tree.set_level(Some("1".into()), level(&[2, 3], &[2, 1]), 0, 0);
        assert_eq!(
            tree.page(1, 3, 1),
            vec![vec![Value::Int64(2), Value::Int64(3), Value::Int64(7)]]
        );
    }
}
