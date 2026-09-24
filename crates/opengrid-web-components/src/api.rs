//! The public surface, frozen (plan point 39).
//!
//! The API of this crate is not its Rust items — every module is `pub(crate)`.
//! It is the **DOM**: three custom elements, their attributes, the events they
//! fire, the parts a page may style, the custom properties it may set, and the
//! keys it may translate. Plus the eight exported functions.
//!
//! This module writes that surface down as data and a test compares it against
//! a list that a human maintains. A name that changes shows up in the diff of
//! that list, where it can be argued about, instead of quietly in a release.
//!
//! **Adding to the list is a decision, not a formality.** Everything in it is a
//! promise to somebody who does not have this repository.

use crate::{grid, pivot, table};

/// Every name a page can rely on, in one string.
fn surface() -> String {
    let mut out = String::new();

    out.push_str("elements\n");
    for tag in [table::TABLE_TAG, grid::GRID_TAG, pivot::PIVOT_TAG] {
        out.push_str(&format!("  {tag}\n"));
    }

    out.push_str("\nattributes\n");
    for (tag, observed) in [
        (table::TABLE_TAG, table::OBSERVED),
        (grid::GRID_TAG, grid::OBSERVED),
        (pivot::PIVOT_TAG, pivot::OBSERVED),
    ] {
        let mut names: Vec<&str> = observed.to_vec();
        names.sort_unstable();
        out.push_str(&format!("  {tag}: {}\n", names.join(" ")));
    }

    out.push_str("\nevents\n");
    for event in [
        crate::grid_element_events::SELECTION_EVENT,
        crate::grid_element_events::CELL_EVENT,
        crate::grid_element_events::VIEW_EVENT,
    ] {
        out.push_str(&format!("  {event}\n"));
    }

    out.push_str("\nfunctions\n");
    for name in [
        "register",
        "set_provider",
        "set_texts",
        "set_formats",
        "set_choices",
        "get_view",
        "set_view",
        "set_columns",
    ] {
        out.push_str(&format!("  {name}\n"));
    }

    // Two groups, because they are two promises: a page *sets* the first and
    // may override the second, which the grid otherwise computes for it.
    out.push_str("\ncustom properties (set)\n");
    out.push_str(&format!("  {}\n", grid::SET_TOKENS.join(" ")));
    out.push_str("\ncustom properties (computed)\n");
    out.push_str(&format!("  {}\n", grid::COMPUTED_TOKENS.join(" ")));

    out.push_str("\nparts\n");
    out.push_str(&format!("  {}\n", parts().join(" ")));

    out.push_str("\ntext keys\n");
    let mut keys: Vec<&str> = crate::texts::KEYS.to_vec();
    keys.sort_unstable();
    out.push_str(&format!("  {}\n", keys.join(" ")));

    out
}

/// Every `part` the three elements write, gathered by building them.
///
/// Computed rather than listed: a part that exists only in the renderer is one
/// a page can already style, and a hand-kept list would not know about it.
fn parts() -> Vec<String> {
    use opengrid_types::{DataType, Field, FieldName, Schema};
    use opengrid_web_core::patch::{NodeAllocator, Patch, PatchBuffer};

    let schema = Schema::new(vec![
        Field::new(FieldName::new("customer").unwrap(), DataType::Utf8),
        Field::new(FieldName::new("qty").unwrap(), DataType::Int64),
    ]);
    let texts = crate::texts::GridTexts::default();
    let declared = vec![("customer".to_owned(), true), ("qty".to_owned(), false)];

    let mut buffer = PatchBuffer::new();
    let mut nodes = NodeAllocator::new();
    grid::build_grid(
        &mut buffer,
        &mut nodes,
        &grid::GridSkeleton {
            label: Some("x"),
            schema: &schema,
            pool: 2,
            texts: &texts,
            declared: &declared,
            presentation: &Default::default(),
            // Built **with** the selection column (point 61): its parts belong
            // to the public surface even though the column is opt-in, and a
            // freeze that only saw the default would not know them.
            selection: true,
            column_menu: true,
            toolbar: true,
            facets: true,
            search: true,
        },
    );
    table::build_table(&mut buffer, &mut nodes, Some("x"), None, None);
    pivot::build_pivot(
        &mut buffer,
        &mut nodes,
        Some("x"),
        None,
        "",
        "ready",
        &texts,
    );

    let mut parts: Vec<String> = buffer
        .patches()
        .iter()
        .filter_map(|patch| match patch {
            Patch::SetAttribute { name, value, .. } if name == "part" => Some(value.clone()),
            _ => None,
        })
        // A part attribute may name more than one (`row total-row`).
        .flat_map(|value| {
            value
                .split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect();
    // Written at render time, not in the skeleton: the editor of point 37 and
    // the total row of a pivot with data.
    parts.push("editor".to_owned());
    parts.push("total-row".to_owned());
    // The column menu of point 64 is built when it opens, not in the skeleton.
    parts.push("column-menu".to_owned());
    parts.push("menu-label".to_owned());
    // The chips of point 65 are drawn from the view by the element.
    parts.push("chip".to_owned());
    parts.push("chip-remove".to_owned());
    parts.push("chips-clear".to_owned());
    // The facet sidebar's contents are drawn from the configuration (point 66),
    // and the toolbar's facet switch exists only once facets are configured.
    for part in [
        "facets-toggle",
        "facets-head",
        "facet-cost",
        "facet",
        "facet-value",
        "facet-count",
        "facet-pills",
        "facet-pill",
        "facet-bounds",
    ] {
        parts.push(part.to_owned());
    }
    parts.sort_unstable();
    parts.dedup();
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The freeze.** Every name a page outside this repository can rely on.
    ///
    /// A failure here is not a bug — it is the diff of a promise. Change the
    /// list when the change is intended, and say why in the commit.
    #[test]
    fn the_public_surface_is_what_it_says_it_is() {
        let expected = "\
elements
  opengrid-table
  opengrid-grid
  opengrid-pivot

attributes
  opengrid-table: columns datasource label
  opengrid-grid: column-menu columns datasource density facets group-by label mode page-size \
search selection toolbar window-size
  opengrid-pivot: columns datasource label rows values

events
  opengrid-selection-change
  opengrid-cell-change
  opengrid-view-change

functions
  register
  set_provider
  set_texts
  set_formats
  set_choices
  get_view
  set_view
  set_columns

custom properties (set)
  --og-font --og-font-mono --og-font-size --og-surface --og-surface-2 --og-ink --og-ink-muted \
--og-line --og-line-strong --og-accent --og-on-accent --og-radius --og-pad --og-focus-width \
--og-row-height --og-header-height --og-filter-height --og-status-height

custom properties (computed)
  --og-accent-soft --og-accent-ink --og-selected --og-hover

parts
  body cell chip chip-remove chips chips-clear column-menu column-menu-button column-toggle \
columns columns-toggle density editor empty empty-reset empty-text facet facet-bounds \
facet-cost facet-count facet-pill facet-pills facet-value facets facets-head facets-toggle \
filter filter-clear filter-operator filter-row-toggle filter-value header layout menu-label \
page-first page-label page-last page-next page-previous pager row search search-hint \
search-input search-list select select-all select-mark sort-direction sort-index status toolbar \
total-row viewport

text keys
  aggregateAvg aggregateCell aggregateCount aggregateGroup aggregateMax aggregateMin \
aggregateNone aggregateRange aggregateSum cellRequired chipRemove chipsClear chipsGroup clear columnAtEdge \
columnHidden columnMenu columnMoved columnShown columnWidth columnsGroup densityComfortable \
densityCompact densityGroup densityNormal empty emptyFiltered emptyReset emptySource emptyValue \
error errorUnknown facetChipValues facetFrom facetQueries facetTo facetsGroup facetsReset \
facetsToggle filterColumn filterGroup filterInvalid filterRemoved filterRowToggle \
filtersCleared groupByColumn groupChip groupCollapsed groupExpanded groupInvalid groupRow \
groupSecondLevel hideColumn lang loading matchesOne matchesOther noValue operatorLabel \
operators pageFirst pageLast pageNext pageOf pagePrevious queryAnd queryMissingValue \
queryUnknownColumn queryWrongOperator rowsOne rowsOther searchChip searchHint searchLabel \
searchPlaceholder searchSuggestions selectAll selectedAll selectionCleared sortAscending \
sortDescending subtotal toolbarGroup total totalRow typeBool typeDate typeInteger typeNumber \
typeText typeTime ungroupColumn valueLabel
";
        assert_eq!(surface(), expected);
    }

    /// Every frozen name, without the section headings and element prefixes.
    fn frozen_names() -> Vec<String> {
        surface()
            .lines()
            .filter(|line| line.starts_with("  "))
            .flat_map(|line| {
                let line = line.trim();
                // `opengrid-grid: columns datasource …` — the tag is its own entry.
                let names = line.split_once(": ").map_or(line, |(_, names)| names);
                names
                    .split_whitespace()
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// The freeze and the documentation are one promise (point 70). Phase E
    /// kept its documentation in the gitignored plan and shipped none; a name
    /// that is frozen but not in `docs/api.md` is that failure again, one name
    /// at a time.
    #[test]
    fn every_frozen_name_is_documented() {
        let docs = include_str!("../../../docs/api.md");
        let missing: Vec<String> = frozen_names()
            .into_iter()
            // A function is documented with its signature, an element as a tag.
            .filter(|name| {
                ![
                    format!("`{name}`"),
                    format!("`{name}("),
                    format!("`<{name}>`"),
                ]
                .iter()
                .any(|form| docs.contains(form.as_str()))
            })
            .collect();
        assert!(
            missing.is_empty(),
            "frozen but not in docs/api.md: {missing:?}"
        );
    }

    /// The parts list of the documentation is the frozen one — no more, no
    /// fewer. A documented part the element does not write is a promise it
    /// breaks the first time a page styles it.
    #[test]
    fn the_documented_parts_are_the_frozen_parts() {
        let docs = include_str!("../../../docs/api.md");
        let list = docs
            .split("**Parts:**")
            .nth(1)
            .and_then(|rest| rest.split("\n\n").next())
            .expect("docs/api.md has a **Parts:** paragraph");
        let mut documented: Vec<String> = list
            .split('`')
            .skip(1)
            .step_by(2)
            .map(str::to_owned)
            .collect();
        documented.sort_unstable();
        assert_eq!(documented, parts());
    }
}
