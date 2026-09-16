//! The empty `<opengrid-table>` skeleton as pure patch data.
//!
//! Point 13 renders the smallest useful thing: a native `<table>` with a
//! `<caption>`, no data, no sorting, no interaction. The table mode of
//! plan/spezifikation/09-accessibility.md §Zwei Rendering-Modi is the safe base
//! — native HTML, maximum semantics — and point 14 fills it with real columns
//! and rows.
//!
//! This module deliberately computes [`Patch`]es instead of touching the DOM:
//! the same function runs on the host in a unit test and in the browser through
//! the renderer.

use opengrid_web_core::element::{LABEL_ATTRIBUTE, mirror_label};
use opengrid_web_core::patch::{NodeAllocator, NodeId, Patch, PatchBuffer};

/// The custom element name (E1).
pub const TABLE_TAG: &str = "opengrid-table";

/// The host attributes the element reacts to.
pub const OBSERVED: &[&str] = &[LABEL_ATTRIBUTE];

/// The two nodes [`build_empty_table`] creates, so a later cycle can patch them
/// in place (point 14).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TableNodes {
    /// The `<table>`.
    pub table: NodeId,
    /// The `<caption>`.
    pub caption: NodeId,
}

/// Appends the empty table skeleton to `buffer` and answers its nodes.
///
/// The caption carries the host `label`; the host label is also mirrored to the
/// table's `aria-label` when present (E8/R6).
pub fn build_empty_table(
    buffer: &mut PatchBuffer,
    nodes: &mut NodeAllocator,
    label: Option<&str>,
) -> TableNodes {
    let table = nodes.alloc();
    buffer.push(Patch::CreateElement {
        node: table,
        tag: "table".to_owned(),
    });
    if let Some((name, value)) = mirror_label(label) {
        buffer.push(Patch::SetAttribute {
            node: table,
            name: name.to_owned(),
            value: value.to_owned(),
        });
    }

    let caption = nodes.alloc();
    buffer.push(Patch::CreateElement {
        node: caption,
        tag: "caption".to_owned(),
    });
    buffer.push(Patch::SetText {
        node: caption,
        text: label.unwrap_or_default().to_owned(),
    });
    buffer.push(Patch::AppendChild {
        parent: table,
        child: caption,
    });
    buffer.push(Patch::AppendChild {
        parent: NodeId::ROOT,
        child: table,
    });

    TableNodes { table, caption }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opengrid_web_core::patch::{NodeAllocator, PatchBuffer};

    /// The skeleton is one patch list: create table, label it, create the
    /// caption, set its text, append both — no DOM work per node.
    #[test]
    fn a_labelled_empty_table_is_one_patch_list() {
        let mut nodes = NodeAllocator::new();
        let mut buffer = PatchBuffer::new();
        let table = build_empty_table(&mut buffer, &mut nodes, Some("Bestellungen"));

        assert_eq!(
            buffer.patches(),
            [
                Patch::CreateElement {
                    node: table.table,
                    tag: "table".to_owned()
                },
                Patch::SetAttribute {
                    node: table.table,
                    name: "aria-label".to_owned(),
                    value: "Bestellungen".to_owned()
                },
                Patch::CreateElement {
                    node: table.caption,
                    tag: "caption".to_owned()
                },
                Patch::SetText {
                    node: table.caption,
                    text: "Bestellungen".to_owned()
                },
                Patch::AppendChild {
                    parent: table.table,
                    child: table.caption
                },
                Patch::AppendChild {
                    parent: NodeId::ROOT,
                    child: table.table
                },
            ]
        );
    }

    /// Without a label there is no `aria-label`, and the caption is empty.
    #[test]
    fn an_unlabelled_table_has_no_aria_label() {
        let mut nodes = NodeAllocator::new();
        let mut buffer = PatchBuffer::new();
        build_empty_table(&mut buffer, &mut nodes, None);

        assert!(
            !buffer
                .patches()
                .iter()
                .any(|patch| matches!(patch, Patch::SetAttribute { .. }))
        );
    }
}
