//! What every renderer in this crate needs, whichever elements are built in.
//!
//! The three renderers (`table`, `grid`, `pivot`) each build a shadow tree out
//! of patches, and each of them needs the same two things: a way to create an
//! element, and the vocabulary of a sortable header. Until plan point 40 that
//! vocabulary lived in [`crate::grid`], and `element` existed three times,
//! byte for byte identical — which meant a module built **without** the grid
//! still had to compile the grid, because `table` and `texts` reached into it
//! for a glyph and a list of operator tokens.
//!
//! So this module is the seam the feature split needs: it is always compiled,
//! it depends on nothing above it, and it holds only what more than one
//! renderer uses.

use opengrid_web_core::patch::{NodeAllocator, NodeId, Patch, PatchBuffer};

/// The glyph marking an ascending column in the header (point 49).
///
/// A filled triangle is the convention in data grids, exists in every font,
/// scales as text at 400% zoom and survives `forced-colors` because it *is*
/// text. It is language-independent, so it needs no entry in the text API of
/// point 48.
pub const ASCENDING_GLYPH: &str = "▲";

/// The glyph marking a descending column in the header (point 49).
pub const DESCENDING_GLYPH: &str = "▼";

/// The operators the type-agnostic filter row offers, in display order.
///
/// The wire names are exactly the query's (plan/spezifikation/02-query-modell.md
/// §Operatoren V1). It lives here rather than beside the filter row because
/// [`crate::texts`] carries one default word per token and asserts at compile
/// time that the two lists stay the same length — and `texts` is compiled even
/// when the grid is not.
pub const FILTER_OPERATORS: &[&str] = &[
    "contains",
    "starts_with",
    "eq",
    "ne",
    "gt",
    "gte",
    "lt",
    "lte",
    "is_null",
    "is_not_null",
];

/// Creates an element and appends it to `parent`, in patch order.
pub(crate) fn element(
    buffer: &mut PatchBuffer,
    nodes: &mut NodeAllocator,
    parent: Option<NodeId>,
    tag: &str,
) -> NodeId {
    let node = nodes.alloc();
    buffer.push(Patch::CreateElement {
        node,
        tag: tag.to_owned(),
    });
    if let Some(parent) = parent {
        buffer.push(Patch::AppendChild {
            parent,
            child: node,
        });
    }
    node
}

/// A decorative span inside a header button, hidden from the accessibility tree.
///
/// The sort direction is already in `aria-sort`, so the glyph is `aria-hidden`:
/// announcing it twice is worse than not announcing it at all.
pub(crate) fn marker(
    buffer: &mut PatchBuffer,
    nodes: &mut NodeAllocator,
    parent: NodeId,
    part: &str,
) -> NodeId {
    let span = element(buffer, nodes, Some(parent), "span");
    buffer.push(Patch::SetAttribute {
        node: span,
        name: "part".to_owned(),
        value: part.to_owned(),
    });
    buffer.push(Patch::SetAttribute {
        node: span,
        name: "aria-hidden".to_owned(),
        value: "true".to_owned(),
    });
    span
}
