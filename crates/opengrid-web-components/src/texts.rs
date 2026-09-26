//! The texts the components write themselves (plan point 48).
//!
//! Everything a user reads that does not come from the data lives here: the
//! status line, the labels of the filter row, the operator names. Two rules
//! shape the module.
//!
//! * **One language, English.** The rest of the public API is English
//!   (`window-size`, `datasource`, the part names, the events), so the built-in
//!   texts are too. Before this point they were half German and half English,
//!   which a screen reader announced in whatever voice the document's `lang`
//!   selected (WCAG 3.1.2).
//! * **The page has the last word.** `set_texts(host, texts)` overrides any
//!   subset of them — the same seam as `set_provider`, so a page wires texts and
//!   data the same way. A `lang` given with them is written onto the component's
//!   content, so the announcement is spoken in the language of the text and not
//!   of the page.
//!
//! What is deliberately *not* here: the diagnoses of engine and provider
//! ("result has no total_count"). They are developer-facing and travel behind a
//! translated sentence, as the `{cause}` of [`GridTexts::error`].
//!
//! The struct and its substitution are portable — they are unit-tested on the
//! host; only the per-host registry and the JS conversion are `wasm32`-only.

/// The texts one component instance renders.
///
/// Every field is a template; `{count}`, `{column}` and `{cause}` are the only
/// placeholders, and each field documents which of them it may use. A field the
/// page does not override keeps the English default of [`Default`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridTexts {
    /// BCP 47 language tag written onto the elements that carry **these** texts
    /// — the filter row and the status line — or empty to leave the document's
    /// language in place.
    ///
    /// Deliberately not on a wrapper around the table: the cells and the column
    /// headers are the page's data, in the page's language, and declaring them
    /// English would be the very WCAG 3.1.2 failure this point removes.
    pub lang: String,
    /// While a query runs.
    pub loading: String,
    /// Exactly one matching row. May use `{count}`.
    pub matches_one: String,
    /// More than one matching row. May use `{count}`.
    pub matches_other: String,
    /// No matching row. The zero case has its own state, so no placeholder.
    pub empty: String,
    /// A failed query. Should use `{cause}` — the untranslated diagnosis.
    pub error: String,
    /// A failed query whose cause is empty.
    pub error_unknown: String,
    /// Accessible name of the filter row.
    pub filter_group: String,
    /// Accessible name of a column's operator control. May use `{column}`.
    pub operator_label: String,
    /// Accessible name of a column's value input. May use `{column}`.
    pub value_label: String,
    /// The button that empties the filter row.
    pub clear: String,
    /// Accessible name of the selection column's header (point 61).
    ///
    /// A word, not the glyph: "✓" read aloud is not a promise anybody can act
    /// on, and the cell it names selects **every matching row**, not the page.
    pub select_all: String,
    /// Said after the header cell selected or cleared everything. May use
    /// `{count}`.
    pub selected_all: String,
    /// A group row (point 62). May use `{column}`, `{value}` and `{rows}`.
    ///
    /// One sentence, not three cells: a group header is read as one thing, and
    /// "country: DE (52 rows)" is what a reader needs before deciding to open it.
    pub group_row: String,
    /// The size of a group, singular.
    pub rows_one: String,
    /// The size of a group. May use `{count}`.
    pub rows_other: String,
    /// Said when a group opens. May use `{group}` and `{rows}`.
    pub group_expanded: String,
    /// Said when a group closes. May use `{group}`.
    pub group_collapsed: String,
    /// A `group-by` the grid refuses. May use `{column}`.
    pub group_invalid: String,
    /// The grand total row (point 63). May use `{rows}`.
    pub total_row: String,
    /// What an aggregate cell is called, for a screen reader (point 63). May use
    /// `{aggregate}` and `{value}`.
    ///
    /// The cell *shows* "Σ 1,234.00" — the glyph drawn by the stylesheet with an
    /// empty alternative — and *says* "Sum: 1,234.00". A glyph read aloud is
    /// "sigma", which is not what anybody meant.
    pub aggregate_cell: String,
    /// The names of the five aggregates.
    pub aggregate_sum: String,
    pub aggregate_avg: String,
    pub aggregate_count: String,
    pub aggregate_min: String,
    pub aggregate_max: String,
    /// The range of a column in a group row, "from – to" (F7).
    pub aggregate_range: String,
    /// The column menu (point 64). May use `{column}`.
    pub column_menu: String,
    pub sort_ascending: String,
    pub sort_descending: String,
    /// The entry that jumps to the column's filter field.
    pub filter_column: String,
    /// The group of aggregate choices in the menu.
    pub aggregate_group: String,
    pub aggregate_none: String,
    pub group_by_column: String,
    pub group_second_level: String,
    pub ungroup_column: String,
    pub hide_column: String,
    /// The toolbar above the grid (point 65).
    pub toolbar_group: String,
    pub filter_row_toggle: String,
    pub density_group: String,
    pub density_compact: String,
    pub density_normal: String,
    pub density_comfortable: String,
    /// The group of active-filter chips.
    pub chips_group: String,
    /// A chip's remove button. May use `{filter}`.
    pub chip_remove: String,
    pub chips_clear: String,
    /// Said when one chip was removed. May use `{filter}`.
    pub filter_removed: String,
    /// Said when "Remove all" was used.
    pub filters_cleared: String,
    /// The grouping chip. May use `{columns}`.
    pub group_chip: String,
    /// The facet sidebar and its switch (point 66).
    pub facets_group: String,
    pub facets_toggle: String,
    pub facets_reset: String,
    /// The two bounds of a range or a period.
    pub facet_from: String,
    pub facet_to: String,
    /// What the counts cost. May use `{count}`.
    pub facet_queries: String,
    /// A facet chip with more than one value. May use `{column}` and `{values}`.
    pub facet_chip_values: String,
    /// The search field (point 67).
    pub search_label: String,
    pub search_placeholder: String,
    /// The word that joins two clauses, in the page's language.
    pub query_and: String,
    /// The hint shown when the input reads as a filter.
    pub search_hint: String,
    /// The list of column suggestions.
    pub search_suggestions: String,
    /// The type shown beside a suggested column (F8, decided 2026-09-24): one
    /// word per kind of column, so a German page does not show "integer".
    pub type_text: String,
    pub type_bool: String,
    pub type_integer: String,
    pub type_number: String,
    pub type_date: String,
    pub type_time: String,
    /// The free-text chip. May use `{text}`.
    pub search_chip: String,
    /// Why an expression did not parse. May use `{column}`, `{operator}`, `{value}`.
    pub query_unknown_column: String,
    pub query_missing_value: String,
    pub query_wrong_operator: String,
    /// The empty state (point 68): no rows because of the filters …
    pub empty_filtered: String,
    /// … and no rows at all.
    pub empty_source: String,
    /// The button that clears every filter, facet and search.
    pub empty_reset: String,
    /// A filter input the column cannot hold (point 51). May use `{column}` and
    /// `{value}`.
    pub filter_invalid: String,
    /// A required cell was cleared (plan point 37). May use `{column}`.
    pub cell_required: String,
    /// A column was resized. May use `{column}` and `{width}` (plan point 36).
    pub column_width: String,
    /// A column was moved. May use `{column}`, `{position}` and `{count}`.
    pub column_moved: String,
    /// A column is already at the first or last place. May use `{column}`.
    pub column_at_edge: String,
    /// A column was hidden. May use `{column}`, `{visible}` and `{count}`.
    pub column_hidden: String,
    /// A column was shown again. Same placeholders.
    pub column_shown: String,
    /// Accessible name of the column-visibility group.
    pub columns_group: String,
    /// The four paging buttons (plan point 38).
    pub page_first: String,
    pub page_previous: String,
    pub page_next: String,
    pub page_last: String,
    /// Where the reader is. May use `{page}` and `{pages}`, both 1-based.
    pub page_of: String,
    /// Appended to the status line when sorting or filtering dropped a
    /// selection (point 35).
    ///
    /// A selection that vanishes without a word is a trap: the next action
    /// would apply to nothing, or to something else.
    pub selection_cleared: String,
    /// The header of a pivot group whose dimension value is NULL.
    ///
    /// A header cell must not be empty: a sighted reader sees a blank and
    /// understands "no country", a screen reader announces nothing at all.
    pub no_value: String,
    /// The header of a pivot group whose dimension value is the **empty
    /// string** — a different group from NULL (rule S14), and it has to look
    /// and sound different too.
    pub empty_value: String,
    /// The row header of a pivot's grand total (plan point 32).
    pub total: String,
    /// The row header of a pivot's subtotal. May use `{value}` — the value of
    /// the dimension the subtotal closes.
    pub subtotal: String,
    /// The readable names of the filter operators, in the order of
    /// [`FILTER_OPERATORS`](crate::shared::FILTER_OPERATORS). The `value` of each
    /// option stays the wire token, so the query is unaffected.
    pub operators: Vec<String>,
}

/// Every key `set_texts` accepts (plan point 39).
///
/// Written down once so the API freeze can check it: a key added to the struct
/// without one here is a key nobody outside this repository can discover.
#[cfg(test)]
pub(crate) const KEYS: &[&str] = &[
    "lang",
    "loading",
    "matchesOne",
    "matchesOther",
    "empty",
    "error",
    "errorUnknown",
    "filterGroup",
    "operatorLabel",
    "valueLabel",
    "clear",
    "selectAll",
    "selectedAll",
    "groupRow",
    "rowsOne",
    "rowsOther",
    "groupExpanded",
    "groupCollapsed",
    "groupInvalid",
    "totalRow",
    "aggregateCell",
    "aggregateSum",
    "aggregateAvg",
    "aggregateCount",
    "aggregateMin",
    "aggregateMax",
    "aggregateRange",
    "columnMenu",
    "sortAscending",
    "sortDescending",
    "filterColumn",
    "aggregateGroup",
    "aggregateNone",
    "groupByColumn",
    "groupSecondLevel",
    "ungroupColumn",
    "hideColumn",
    "toolbarGroup",
    "filterRowToggle",
    "densityGroup",
    "densityCompact",
    "densityNormal",
    "densityComfortable",
    "chipsGroup",
    "chipRemove",
    "chipsClear",
    "filterRemoved",
    "filtersCleared",
    "groupChip",
    "facetsGroup",
    "facetsToggle",
    "facetsReset",
    "facetFrom",
    "facetTo",
    "facetQueries",
    "facetChipValues",
    "searchLabel",
    "searchPlaceholder",
    "queryAnd",
    "searchHint",
    "searchSuggestions",
    "typeText",
    "typeBool",
    "typeInteger",
    "typeNumber",
    "typeDate",
    "typeTime",
    "searchChip",
    "queryUnknownColumn",
    "queryMissingValue",
    "queryWrongOperator",
    "emptyFiltered",
    "emptySource",
    "emptyReset",
    "filterInvalid",
    "cellRequired",
    "columnWidth",
    "columnMoved",
    "columnAtEdge",
    "columnHidden",
    "columnShown",
    "columnsGroup",
    "pageFirst",
    "pagePrevious",
    "pageNext",
    "pageLast",
    "pageOf",
    "selectionCleared",
    "noValue",
    "emptyValue",
    "total",
    "subtotal",
    "operators",
];

/// The readable operator names, in the order of
/// [`FILTER_OPERATORS`](crate::shared::FILTER_OPERATORS).
///
/// `gte` is not a word. The wire token stays the option's `value`; only what the
/// user reads changes.
pub const DEFAULT_OPERATORS: &[&str] = &[
    "contains",
    "starts with",
    "is",
    "is not",
    "greater than",
    "greater or equal",
    "less than",
    "less or equal",
    // Deliberately not "is empty": an empty string **is** a value (rule S14),
    // and calling the absence of a value "empty" would merge the two.
    "has no value",
    "has a value",
];

/// The labels are indexed by the wire tokens' position, so the two lists must
/// have the same length — otherwise an added operator would silently render its
/// raw token.
const _: () = assert!(DEFAULT_OPERATORS.len() == crate::shared::FILTER_OPERATORS.len());

impl Default for GridTexts {
    fn default() -> Self {
        Self {
            lang: "en".to_owned(),
            loading: "Loading …".to_owned(),
            matches_one: "{count} match".to_owned(),
            matches_other: "{count} matches".to_owned(),
            empty: "No matches".to_owned(),
            error: "The data could not be loaded: {cause}".to_owned(),
            error_unknown: "The data could not be loaded.".to_owned(),
            filter_group: "Filter".to_owned(),
            operator_label: "{column} operator".to_owned(),
            value_label: "{column} value".to_owned(),
            clear: "Clear".to_owned(),
            select_all: "Select all matching rows".to_owned(),
            selected_all: "{count} rows selected".to_owned(),
            group_row: "{column}: {value} ({rows})".to_owned(),
            rows_one: "1 row".to_owned(),
            rows_other: "{count} rows".to_owned(),
            group_expanded: "{group} expanded, {rows}".to_owned(),
            group_collapsed: "{group} collapsed".to_owned(),
            group_invalid: "Cannot group by {column}".to_owned(),
            total_row: "Total ({rows})".to_owned(),
            aggregate_cell: "{aggregate}: {value}".to_owned(),
            aggregate_sum: "Sum".to_owned(),
            aggregate_avg: "Average".to_owned(),
            aggregate_count: "Count".to_owned(),
            aggregate_min: "Minimum".to_owned(),
            aggregate_max: "Maximum".to_owned(),
            aggregate_range: "Range".to_owned(),
            column_menu: "{column} column menu".to_owned(),
            sort_ascending: "Sort ascending".to_owned(),
            sort_descending: "Sort descending".to_owned(),
            filter_column: "Filter …".to_owned(),
            aggregate_group: "Aggregate in groups".to_owned(),
            aggregate_none: "No aggregate".to_owned(),
            group_by_column: "Group by this column".to_owned(),
            group_second_level: "Group as second level".to_owned(),
            ungroup_column: "Remove this grouping".to_owned(),
            hide_column: "Hide column".to_owned(),
            toolbar_group: "Grid tools".to_owned(),
            filter_row_toggle: "Filter row".to_owned(),
            density_group: "Density".to_owned(),
            density_compact: "Compact".to_owned(),
            density_normal: "Normal".to_owned(),
            density_comfortable: "Comfortable".to_owned(),
            chips_group: "Active filters".to_owned(),
            chip_remove: "Remove {filter}".to_owned(),
            chips_clear: "Remove all".to_owned(),
            filter_removed: "{filter} removed".to_owned(),
            filters_cleared: "All filters removed".to_owned(),
            group_chip: "Grouped by {columns}".to_owned(),
            facets_group: "Facets".to_owned(),
            facets_toggle: "Facets".to_owned(),
            facets_reset: "Reset facets".to_owned(),
            facet_from: "From".to_owned(),
            facet_to: "To".to_owned(),
            facet_queries: "Counted with {count} queries".to_owned(),
            facet_chip_values: "{column} is one of {values}".to_owned(),
            search_label: "Search or filter".to_owned(),
            search_placeholder: "Search, or filter: country = DE and amount \u{2265} 10".to_owned(),
            query_and: "and".to_owned(),
            search_hint: "Query \u{00B7} Enter".to_owned(),
            search_suggestions: "Columns".to_owned(),
            type_text: "text".to_owned(),
            type_bool: "yes/no".to_owned(),
            type_integer: "integer".to_owned(),
            type_number: "number".to_owned(),
            type_date: "date".to_owned(),
            type_time: "time".to_owned(),
            search_chip: "Text contains \u{201C}{text}\u{201D}".to_owned(),
            query_unknown_column: "{column} is not a column of this grid".to_owned(),
            query_missing_value: "{column}: the value is missing".to_owned(),
            query_wrong_operator: "{column} does not take {operator}".to_owned(),
            empty_filtered: "No row matches these filters.".to_owned(),
            empty_source: "There are no rows.".to_owned(),
            empty_reset: "Reset filters".to_owned(),
            filter_invalid: "{column}: {value} is not a value for this column".to_owned(),
            cell_required: "{column} needs a value".to_owned(),
            column_width: "{column} is {width} pixels wide".to_owned(),
            column_moved: "{column} moved to position {position} of {count}".to_owned(),
            column_at_edge: "{column} is already at the end".to_owned(),
            column_hidden: "{column} hidden, {visible} of {count} columns shown".to_owned(),
            column_shown: "{column} shown, {visible} of {count} columns shown".to_owned(),
            columns_group: "Columns".to_owned(),
            page_first: "First page".to_owned(),
            page_previous: "Previous page".to_owned(),
            page_next: "Next page".to_owned(),
            page_last: "Last page".to_owned(),
            page_of: "Page {page} of {pages}".to_owned(),
            selection_cleared: "Selection cleared".to_owned(),
            no_value: "(no value)".to_owned(),
            empty_value: "(empty)".to_owned(),
            total: "Total".to_owned(),
            subtotal: "Total {value}".to_owned(),
            operators: DEFAULT_OPERATORS
                .iter()
                .map(|name| (*name).to_owned())
                .collect(),
        }
    }
}

impl GridTexts {
    /// The result count line for `count` matching rows (`count >= 1`).
    pub fn matches(&self, count: u64) -> String {
        let template = if count == 1 {
            &self.matches_one
        } else {
            &self.matches_other
        };
        fill(template, "count", &count.to_string())
    }

    /// The sentence for a failed query. An empty cause takes
    /// [`error_unknown`](Self::error_unknown), so no template ever renders a
    /// dangling separator.
    pub fn error(&self, cause: &str) -> String {
        let cause = cause.trim();
        if cause.is_empty() {
            self.error_unknown.clone()
        } else {
            fill(&self.error, "cause", cause)
        }
    }

    /// The accessible name of `column`'s operator control.
    pub fn operator_label(&self, column: &str) -> String {
        fill(&self.operator_label, "column", column)
    }

    /// The accessible name of `column`'s value input.
    pub fn value_label(&self, column: &str) -> String {
        fill(&self.value_label, "column", column)
    }

    /// The sentence for a filter input the column cannot hold.
    pub fn filter_invalid(&self, column: &str, value: &str) -> String {
        fill(
            &fill(&self.filter_invalid, "column", column),
            "value",
            value,
        )
    }

    /// A cell that may not be empty.
    pub fn cell_required(&self, column: &str) -> String {
        fill(&self.cell_required, "column", column)
    }

    /// What a column operation did, for the status line (plan point 36).
    ///
    /// A width, a move or a hidden column is a change only the sighted see, so
    /// each one gets a sentence.
    /// "1 row" or "{count} rows" (point 62).
    pub fn rows(&self, count: u64) -> String {
        if count == 1 {
            self.rows_one.clone()
        } else {
            fill(&self.rows_other, "count", &count.to_string())
        }
    }

    /// The text of a group row (point 62).
    pub fn group_row(&self, column: &str, value: &str, count: u64) -> String {
        fill(
            &fill(&fill(&self.group_row, "column", column), "value", value),
            "rows",
            &self.rows(count),
        )
    }

    /// What is said when a group opens or closes (point 62).
    pub fn group_toggled(&self, group: &str, count: u64, open: bool) -> String {
        if open {
            fill(
                &fill(&self.group_expanded, "group", group),
                "rows",
                &self.rows(count),
            )
        } else {
            fill(&self.group_collapsed, "group", group)
        }
    }

    /// A chip's remove button (point 65).
    pub fn chip_remove(&self, filter: &str) -> String {
        fill(&self.chip_remove, "filter", filter)
    }

    /// What is said when a chip was removed (point 65).
    pub fn filter_removed(&self, filter: &str) -> String {
        fill(&self.filter_removed, "filter", filter)
    }

    /// Why an expression in the search field did not parse (point 67).
    #[cfg(feature = "grid")]
    pub fn query_problem(&self, problem: &crate::search::Problem) -> String {
        use crate::search::Problem;
        match problem {
            Problem::UnknownColumn(column) => fill(&self.query_unknown_column, "column", column),
            Problem::MissingValue(column) => fill(&self.query_missing_value, "column", column),
            Problem::WrongOperator { column, operator } => fill(
                &fill(&self.query_wrong_operator, "column", column),
                "operator",
                operator,
            ),
            Problem::WrongValue { column, value } => self.filter_invalid(column, value),
        }
    }

    /// The free-text chip (point 67).
    pub fn search_chip(&self, text: &str) -> String {
        fill(&self.search_chip, "text", text)
    }

    /// What the facet counts cost (point 66).
    pub fn facet_queries(&self, count: usize) -> String {
        fill(&self.facet_queries, "count", &count.to_string())
    }

    /// A chip for a facet with several values (point 66).
    pub fn facet_chip_values(&self, column: &str, values: &str) -> String {
        fill(
            &fill(&self.facet_chip_values, "column", column),
            "values",
            values,
        )
    }

    /// The grouping chip (point 65).
    pub fn group_chip(&self, columns: &str) -> String {
        fill(&self.group_chip, "columns", columns)
    }

    /// The name of one density (point 65).
    pub fn density(&self, name: &str) -> &str {
        match name {
            "compact" => &self.density_compact,
            "comfortable" => &self.density_comfortable,
            _ => &self.density_normal,
        }
    }

    /// The column menu's accessible name (point 64).
    pub fn column_menu(&self, column: &str) -> String {
        fill(&self.column_menu, "column", column)
    }

    /// The name of one aggregate, as a menu entry and a cell say it.
    #[cfg(feature = "grid")]
    pub fn aggregate_name(&self, summary: crate::presentation::Summary) -> &str {
        use crate::presentation::Summary;
        use opengrid_query::AggregateFn;
        match summary {
            Summary::Fn(AggregateFn::Sum) => &self.aggregate_sum,
            Summary::Fn(AggregateFn::Avg) => &self.aggregate_avg,
            Summary::Fn(AggregateFn::Count) => &self.aggregate_count,
            Summary::Fn(AggregateFn::Min) => &self.aggregate_min,
            Summary::Fn(AggregateFn::Max) => &self.aggregate_max,
            Summary::Range => &self.aggregate_range,
        }
    }

    /// The grand total row's label (point 63).
    pub fn total_row(&self, count: u64) -> String {
        fill(&self.total_row, "rows", &self.rows(count))
    }

    /// What an aggregate cell says (point 63).
    #[cfg(feature = "grid")]
    pub fn aggregate_cell(&self, summary: crate::presentation::Summary, value: &str) -> String {
        let name = self.aggregate_name(summary);
        fill(
            &fill(&self.aggregate_cell, "aggregate", name),
            "value",
            value,
        )
    }

    /// A `group-by` the grid refuses (point 62).
    pub fn group_invalid(&self, column: &str) -> String {
        fill(&self.group_invalid, "column", column)
    }

    /// What the selection column's header says after it acted (point 61).
    pub fn selected_all(&self, count: u64) -> String {
        fill(&self.selected_all, "count", &count.to_string())
    }

    pub fn column_width(&self, column: &str, width: u32) -> String {
        fill(
            &fill(&self.column_width, "column", column),
            "width",
            &width.to_string(),
        )
    }

    /// A column that reached its new place.
    pub fn column_moved(&self, column: &str, position: u64, count: u64) -> String {
        fill(
            &fill(
                &fill(&self.column_moved, "column", column),
                "position",
                &position.to_string(),
            ),
            "count",
            &count.to_string(),
        )
    }

    /// A column that cannot move any further.
    pub fn column_at_edge(&self, column: &str) -> String {
        fill(&self.column_at_edge, "column", column)
    }

    /// A column that was hidden or shown again.
    pub fn column_visibility(
        &self,
        column: &str,
        hidden: bool,
        visible: u64,
        count: u64,
    ) -> String {
        let template = if hidden {
            &self.column_hidden
        } else {
            &self.column_shown
        };
        fill(
            &fill(
                &fill(template, "column", column),
                "visible",
                &visible.to_string(),
            ),
            "count",
            &count.to_string(),
        )
    }

    /// Where the reader is, both numbers 1-based.
    pub fn page_of(&self, page: u64, pages: u64) -> String {
        fill(
            &fill(&self.page_of, "page", &page.to_string()),
            "pages",
            &pages.to_string(),
        )
    }

    /// A dimension value as a **header** reads it.
    ///
    /// NULL and the empty string are two different groups (S10, S14) and two
    /// different words; neither may render as an empty header cell.
    pub fn dimension(&self, value: Option<&str>) -> String {
        match value {
            None => self.no_value.clone(),
            Some("") => self.empty_value.clone(),
            Some(text) => text.to_owned(),
        }
    }

    /// The row header of the subtotal that closes `value`.
    ///
    /// A subtotal has to be **readable** as one, not only shaded: colour alone
    /// is not information (WCAG 1.4.1), and a screen reader announces this text
    /// where a sighted reader sees the shading.
    pub fn subtotal(&self, value: &str) -> String {
        fill(&self.subtotal, "value", value)
    }

    /// The readable name of the operator at `index`, falling back to the wire
    /// token when there is none.
    ///
    /// The list is indexed like
    /// [`FILTER_OPERATORS`](crate::shared::FILTER_OPERATORS) — a compile-time
    /// assertion keeps the two the same length — and an empty entry means "no
    /// label", so a page can translate one operator without restating the rest.
    pub fn operator(&self, index: usize, token: &str) -> String {
        match self.operators.get(index) {
            Some(name) if !name.is_empty() => name.clone(),
            _ => token.to_owned(),
        }
    }
}

/// Replaces every `{name}` in `template`.
fn fill(template: &str, name: &str, value: &str) -> String {
    template.replace(&format!("{{{name}}}"), value)
}

#[cfg(target_arch = "wasm32")]
pub use host::texts;
#[cfg(target_arch = "wasm32")]
pub(crate) use host::{from_js, store};

/// Per-host storage of the texts, keyed by the host id and released when the
/// host is collected (`opengrid_web_core::host`, point 74).
#[cfg(target_arch = "wasm32")]
mod host {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    use opengrid_web_core::host::{existing_id, id as host_id, on_release};
    use wasm_bindgen::JsValue;
    use web_sys::HtmlElement;

    use super::GridTexts;

    thread_local! {
        static TEXTS: RefCell<HashMap<u32, Rc<GridTexts>>> = RefCell::new(HashMap::new());
    }

    fn release(id: u32) {
        TEXTS.with(|map| map.borrow_mut().remove(&id));
    }

    /// Attaches `texts` to `host`, replacing any previous ones.
    pub(crate) fn store(host: &HtmlElement, texts: Rc<GridTexts>) {
        on_release(release);
        let id = host_id(host);
        TEXTS.with(|map| map.borrow_mut().insert(id, texts));
    }

    /// The texts of `host` — the page's, or the English defaults.
    ///
    /// The default is built once per thread: this is read on every rendered
    /// frame, and a grid that never sets texts is the common case.
    pub fn texts(host: &HtmlElement) -> Rc<GridTexts> {
        thread_local! {
            static DEFAULT: Rc<GridTexts> = Rc::new(GridTexts::default());
        }
        let stored =
            existing_id(host).and_then(|id| TEXTS.with(|map| map.borrow().get(&id).cloned()));
        stored.unwrap_or_else(|| DEFAULT.with(Rc::clone))
    }

    /// Reads a JS object into [`GridTexts`], keeping the default of every key it
    /// does not carry.
    ///
    /// A partial object is the normal case: a page that only wants a German
    /// "Clear" should not have to restate the other ten texts. Keys are
    /// camelCase, as JavaScript writes them.
    pub(crate) fn from_js(value: &JsValue) -> GridTexts {
        let mut texts = GridTexts::default();
        let string = |key: &str| -> Option<String> {
            js_sys::Reflect::get(value, &JsValue::from_str(key))
                .ok()
                .and_then(|value| value.as_string())
        };
        let overwrite = |field: &mut String, value: Option<String>| {
            if let Some(value) = value {
                *field = value;
            }
        };
        overwrite(&mut texts.lang, string("lang"));
        overwrite(&mut texts.loading, string("loading"));
        overwrite(&mut texts.matches_one, string("matchesOne"));
        overwrite(&mut texts.matches_other, string("matchesOther"));
        overwrite(&mut texts.empty, string("empty"));
        overwrite(&mut texts.error, string("error"));
        overwrite(&mut texts.error_unknown, string("errorUnknown"));
        overwrite(&mut texts.filter_group, string("filterGroup"));
        overwrite(&mut texts.operator_label, string("operatorLabel"));
        overwrite(&mut texts.value_label, string("valueLabel"));
        overwrite(&mut texts.clear, string("clear"));
        overwrite(&mut texts.select_all, string("selectAll"));
        overwrite(&mut texts.selected_all, string("selectedAll"));
        overwrite(&mut texts.group_row, string("groupRow"));
        overwrite(&mut texts.rows_one, string("rowsOne"));
        overwrite(&mut texts.rows_other, string("rowsOther"));
        overwrite(&mut texts.group_expanded, string("groupExpanded"));
        overwrite(&mut texts.group_collapsed, string("groupCollapsed"));
        overwrite(&mut texts.group_invalid, string("groupInvalid"));
        overwrite(&mut texts.total_row, string("totalRow"));
        overwrite(&mut texts.aggregate_cell, string("aggregateCell"));
        overwrite(&mut texts.aggregate_sum, string("aggregateSum"));
        overwrite(&mut texts.aggregate_avg, string("aggregateAvg"));
        overwrite(&mut texts.aggregate_count, string("aggregateCount"));
        overwrite(&mut texts.aggregate_min, string("aggregateMin"));
        overwrite(&mut texts.aggregate_max, string("aggregateMax"));
        overwrite(&mut texts.aggregate_range, string("aggregateRange"));
        overwrite(&mut texts.column_menu, string("columnMenu"));
        overwrite(&mut texts.sort_ascending, string("sortAscending"));
        overwrite(&mut texts.sort_descending, string("sortDescending"));
        overwrite(&mut texts.filter_column, string("filterColumn"));
        overwrite(&mut texts.aggregate_group, string("aggregateGroup"));
        overwrite(&mut texts.aggregate_none, string("aggregateNone"));
        overwrite(&mut texts.group_by_column, string("groupByColumn"));
        overwrite(&mut texts.group_second_level, string("groupSecondLevel"));
        overwrite(&mut texts.ungroup_column, string("ungroupColumn"));
        overwrite(&mut texts.hide_column, string("hideColumn"));
        overwrite(&mut texts.toolbar_group, string("toolbarGroup"));
        overwrite(&mut texts.filter_row_toggle, string("filterRowToggle"));
        overwrite(&mut texts.density_group, string("densityGroup"));
        overwrite(&mut texts.density_compact, string("densityCompact"));
        overwrite(&mut texts.density_normal, string("densityNormal"));
        overwrite(&mut texts.density_comfortable, string("densityComfortable"));
        overwrite(&mut texts.chips_group, string("chipsGroup"));
        overwrite(&mut texts.chip_remove, string("chipRemove"));
        overwrite(&mut texts.chips_clear, string("chipsClear"));
        overwrite(&mut texts.filter_removed, string("filterRemoved"));
        overwrite(&mut texts.filters_cleared, string("filtersCleared"));
        overwrite(&mut texts.group_chip, string("groupChip"));
        overwrite(&mut texts.facets_group, string("facetsGroup"));
        overwrite(&mut texts.facets_toggle, string("facetsToggle"));
        overwrite(&mut texts.facets_reset, string("facetsReset"));
        overwrite(&mut texts.facet_from, string("facetFrom"));
        overwrite(&mut texts.facet_to, string("facetTo"));
        overwrite(&mut texts.facet_queries, string("facetQueries"));
        overwrite(&mut texts.facet_chip_values, string("facetChipValues"));
        overwrite(&mut texts.search_label, string("searchLabel"));
        overwrite(&mut texts.search_placeholder, string("searchPlaceholder"));
        overwrite(&mut texts.query_and, string("queryAnd"));
        overwrite(&mut texts.search_hint, string("searchHint"));
        overwrite(&mut texts.search_suggestions, string("searchSuggestions"));
        overwrite(&mut texts.type_text, string("typeText"));
        overwrite(&mut texts.type_bool, string("typeBool"));
        overwrite(&mut texts.type_integer, string("typeInteger"));
        overwrite(&mut texts.type_number, string("typeNumber"));
        overwrite(&mut texts.type_date, string("typeDate"));
        overwrite(&mut texts.type_time, string("typeTime"));
        overwrite(&mut texts.search_chip, string("searchChip"));
        overwrite(
            &mut texts.query_unknown_column,
            string("queryUnknownColumn"),
        );
        overwrite(&mut texts.query_missing_value, string("queryMissingValue"));
        overwrite(
            &mut texts.query_wrong_operator,
            string("queryWrongOperator"),
        );
        overwrite(&mut texts.empty_filtered, string("emptyFiltered"));
        overwrite(&mut texts.empty_source, string("emptySource"));
        overwrite(&mut texts.empty_reset, string("emptyReset"));
        overwrite(&mut texts.filter_invalid, string("filterInvalid"));
        overwrite(&mut texts.cell_required, string("cellRequired"));
        overwrite(&mut texts.column_width, string("columnWidth"));
        overwrite(&mut texts.column_moved, string("columnMoved"));
        overwrite(&mut texts.column_at_edge, string("columnAtEdge"));
        overwrite(&mut texts.column_hidden, string("columnHidden"));
        overwrite(&mut texts.column_shown, string("columnShown"));
        overwrite(&mut texts.columns_group, string("columnsGroup"));
        overwrite(&mut texts.page_first, string("pageFirst"));
        overwrite(&mut texts.page_previous, string("pagePrevious"));
        overwrite(&mut texts.page_next, string("pageNext"));
        overwrite(&mut texts.page_last, string("pageLast"));
        overwrite(&mut texts.page_of, string("pageOf"));
        overwrite(&mut texts.selection_cleared, string("selectionCleared"));
        overwrite(&mut texts.no_value, string("noValue"));
        overwrite(&mut texts.empty_value, string("emptyValue"));
        overwrite(&mut texts.total, string("total"));
        overwrite(&mut texts.subtotal, string("subtotal"));

        // `operators` is keyed by the wire token — `{ gte: "greater or equal" }`.
        // A positional array would silently shift every label when one entry is
        // missing or not a string, and it would force a page that wants to
        // translate one operator to restate all eight.
        if let Ok(map) = js_sys::Reflect::get(value, &JsValue::from_str("operators"))
            && map.is_object()
        {
            for (index, token) in crate::shared::FILTER_OPERATORS.iter().enumerate() {
                if let Ok(label) = js_sys::Reflect::get(&map, &JsValue::from_str(token))
                    && let Some(label) = label.as_string()
                    && let Some(slot) = texts.operators.get_mut(index)
                {
                    *slot = label;
                }
            }
        }
        texts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The built-in texts are English — the point of this plan point.
    #[test]
    fn the_defaults_are_english() {
        let texts = GridTexts::default();
        assert_eq!(texts.lang, "en");
        assert_eq!(texts.loading, "Loading …");
        assert_eq!(texts.empty, "No matches");
        assert_eq!(texts.clear, "Clear");
    }

    /// One match is not "1 matches".
    #[test]
    fn the_count_line_has_a_singular() {
        let texts = GridTexts::default();
        assert_eq!(texts.matches(1), "1 match");
        assert_eq!(texts.matches(2), "2 matches");
        assert_eq!(texts.matches(1_234), "1234 matches");
    }

    /// The cause is untranslated, but the sentence around it is not — and an
    /// empty cause never renders a dangling colon.
    #[test]
    fn the_error_carries_its_cause() {
        let texts = GridTexts::default();
        assert_eq!(
            texts.error("unknown source \"orders\""),
            "The data could not be loaded: unknown source \"orders\""
        );
        assert_eq!(texts.error("   "), "The data could not be loaded.");
    }

    /// The refusal names both the column and what was typed — otherwise the
    /// announcement leaves the user guessing which field it means.
    #[test]
    fn an_invalid_filter_names_the_column_and_the_value() {
        let texts = GridTexts::default();
        assert_eq!(
            texts.filter_invalid("qty", "zwei"),
            "qty: zwei is not a value for this column"
        );
    }

    #[test]
    fn labels_name_their_column() {
        let texts = GridTexts::default();
        assert_eq!(texts.operator_label("customer"), "customer operator");
        assert_eq!(texts.value_label("customer"), "customer value");
    }

    /// The user reads a word, the query keeps the token.
    #[test]
    fn operators_read_as_words() {
        let texts = GridTexts::default();
        assert_eq!(texts.operator(0, "contains"), "contains");
        assert_eq!(texts.operator(5, "gte"), "greater or equal");

        // A missing or empty label falls back to the token rather than shifting
        // the others — a label must never name a different operator than the
        // one its `value` sends.
        let sparse = GridTexts {
            operators: vec!["enthält".to_owned(), String::new()],
            ..GridTexts::default()
        };
        assert_eq!(sparse.operator(0, "contains"), "enthält");
        assert_eq!(sparse.operator(1, "starts_with"), "starts_with");
        assert_eq!(sparse.operator(5, "gte"), "gte");
    }

    /// A template without the placeholder is left alone rather than mangled.
    #[test]
    fn a_template_without_a_placeholder_is_kept() {
        let texts = GridTexts {
            matches_other: "Treffer".to_owned(),
            ..GridTexts::default()
        };
        assert_eq!(texts.matches(7), "Treffer");
    }
}
