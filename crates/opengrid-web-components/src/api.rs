//! The public surface, frozen (plan point 39).
//!
//! The API of this crate is not its Rust items — every module is `pub(crate)`.
//! It is the **DOM**: three custom elements, their attributes, the events they
//! fire, the parts a page may style, the custom properties it may set, and the
//! keys it may translate. Plus four exported functions.
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
    ] {
        out.push_str(&format!("  {name}\n"));
    }

    out.push_str("\ncustom properties\n");
    for property in [
        grid::ROW_HEIGHT_PROPERTY,
        grid::HEADER_HEIGHT_PROPERTY,
        grid::FILTER_HEIGHT_PROPERTY,
        grid::STATUS_HEIGHT_PROPERTY,
        grid::BORDER_COLOR_PROPERTY,
        grid::FOCUS_WIDTH_PROPERTY,
    ] {
        out.push_str(&format!("  {property}\n"));
    }

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
        Some("x"),
        &schema,
        2,
        &texts,
        &declared,
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
  opengrid-grid: columns datasource label mode page-size window-size
  opengrid-pivot: columns datasource label rows values

events
  opengrid-selection-change
  opengrid-cell-change

functions
  register
  set_provider
  set_texts
  set_formats
  set_choices

custom properties
  --grid-row-height
  --grid-header-height
  --grid-filter-height
  --grid-status-height
  --grid-border-color
  --grid-focus-width

parts
  cell column-toggle columns columns-toggle editor filter filter-clear filter-operator \
filter-value header layout page-first page-label page-last page-next page-previous pager row \
sort-direction sort-index status total-row viewport

text keys
  cellRequired clear columnAtEdge columnHidden columnMoved columnShown columnWidth \
columnsGroup empty emptyValue error errorUnknown filterGroup filterInvalid lang loading \
matchesOne matchesOther noValue operatorLabel operators pageFirst pageLast pageNext pageOf \
pagePrevious selectionCleared subtotal total valueLabel
";
        assert_eq!(surface(), expected);
    }
}
