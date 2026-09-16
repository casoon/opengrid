//! DOM patches and the per-frame patch buffer (risk R1).
//!
//! The grid does not touch the DOM while it computes a frame. It appends
//! [`Patch`]es to one [`PatchBuffer`] and applies that buffer once
//! (plan/spezifikation/08-rendering.md §DOM-Brücke: "pro Frame eine Patch-Liste,
//! keine Einzelupdates Zelle für Zelle"). The buffer is plain data — it can be
//! built and asserted on the host, without a browser.
//!
//! Node identity is a [`NodeId`], an opaque handle that only means something
//! together with the [`Dom`](crate::renderer::Dom) that created it. Patch
//! computation therefore never holds a `web_sys` type.

/// A handle for a node the [`Dom`](crate::renderer::Dom) knows about.
///
/// [`NodeId::ROOT`] is reserved for the render root (the shadow root of a
/// custom element). [`NodeAllocator`] hands out every other id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(u32);

impl NodeId {
    /// The node the component renders into — its open shadow root (E8).
    pub const ROOT: NodeId = NodeId(0);

    /// The raw index, for diagnostics.
    pub fn index(self) -> u32 {
        self.0
    }
}

/// One DOM operation.
///
/// Applied in order; the order inside a buffer is the order the operations must
/// run in (create a parent before its child). See
/// [`Dom::apply_buffer`](crate::renderer::Dom::apply_buffer).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Patch {
    /// Creates an element and binds it to `node`.
    CreateElement { node: NodeId, tag: String },
    /// Sets (or replaces) an attribute.
    SetAttribute {
        node: NodeId,
        name: String,
        value: String,
    },
    /// Removes an attribute if present.
    RemoveAttribute { node: NodeId, name: String },
    /// Replaces the node's text content.
    SetText { node: NodeId, text: String },
    /// Appends `child` to `parent` (at the end).
    AppendChild { parent: NodeId, child: NodeId },
}

/// The patch list of a single frame.
///
/// One cycle builds one buffer; the grid then applies it in one go. There is no
/// API to apply a patch to the DOM directly, which is what keeps "no DOM access
/// per cell" a structural property rather than a convention.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct PatchBuffer {
    patches: Vec<Patch>,
}

impl PatchBuffer {
    /// An empty frame.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a patch to this frame.
    pub fn push(&mut self, patch: Patch) {
        self.patches.push(patch);
    }

    /// Number of patches in this frame.
    pub fn len(&self) -> usize {
        self.patches.len()
    }

    /// Whether this frame is empty (nothing to apply).
    pub fn is_empty(&self) -> bool {
        self.patches.is_empty()
    }

    /// The patches in application order.
    pub fn patches(&self) -> &[Patch] {
        &self.patches
    }

    /// Empties the buffer, keeping its allocation for the next frame.
    pub fn clear(&mut self) {
        self.patches.clear();
    }

    /// Consumes the buffer and answers its patches.
    pub fn into_patches(self) -> Vec<Patch> {
        self.patches
    }
}

impl Extend<Patch> for PatchBuffer {
    fn extend<I: IntoIterator<Item = Patch>>(&mut self, iter: I) {
        self.patches.extend(iter);
    }
}

/// Hands out [`NodeId`]s for one render tree.
///
/// Starts at 1 because [`NodeId::ROOT`] (0) is the render root the
/// [`Dom`](crate::renderer::Dom) is created with.
#[derive(Debug)]
pub struct NodeAllocator {
    next: u32,
}

impl NodeAllocator {
    /// A fresh allocator; the first id it hands out is `1`.
    pub fn new() -> Self {
        Self { next: 1 }
    }

    /// Reserves the next id.
    pub fn alloc(&mut self) -> NodeId {
        let id = NodeId(self.next);
        self.next += 1;
        id
    }
}

impl Default for NodeAllocator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a plain header + data table as one patch list, the way a render
    /// cycle would (plan point 13 step 1).
    fn build_table(buffer: &mut PatchBuffer, nodes: &mut NodeAllocator, rows: u32, cols: u32) {
        let mut element = |buffer: &mut PatchBuffer, parent: Option<NodeId>, tag: &str| {
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
        };

        let table = element(buffer, Some(NodeId::ROOT), "table");
        let thead = element(buffer, Some(table), "thead");
        let header_row = element(buffer, Some(thead), "tr");
        for _ in 0..cols {
            element(buffer, Some(header_row), "th");
        }

        let tbody = element(buffer, Some(table), "tbody");
        for _ in 0..rows {
            let row = element(buffer, Some(tbody), "tr");
            for _ in 0..cols {
                let cell = element(buffer, Some(row), "td");
                buffer.push(Patch::SetText {
                    node: cell,
                    text: "x".to_owned(),
                });
            }
        }
    }

    /// One render cycle produces exactly one patch list, holding the whole
    /// table — not one buffer or one DOM call per cell.
    #[test]
    fn a_table_is_one_patch_list() {
        let mut nodes = NodeAllocator::new();
        let mut buffer = PatchBuffer::new();
        build_table(&mut buffer, &mut nodes, 2, 2);

        // Elements: table + thead + header row + 2 th + tbody + 2 rows + 4 td = 12.
        // Patches: one create and one append per element, plus one text per data cell.
        assert_eq!(buffer.len(), 2 * 12 + 4);
        assert!(matches!(
            buffer.patches().first(),
            Some(Patch::CreateElement { node, tag }) if node.index() == 1 && tag == "table"
        ));
        assert!(matches!(
            buffer.patches().last(),
            Some(Patch::SetText { text, .. }) if text == "x"
        ));
    }

    /// A round of cell updates is still a single frame; the buffer exists
    /// precisely so a hundred cells are one apply, not a hundred DOM calls.
    #[test]
    fn many_cell_updates_share_one_buffer() {
        let mut nodes = NodeAllocator::new();
        let mut buffer = PatchBuffer::new();
        let cell = nodes.alloc();
        buffer.push(Patch::CreateElement {
            node: cell,
            tag: "td".to_owned(),
        });
        for index in 0..100 {
            buffer.push(Patch::SetText {
                node: cell,
                text: index.to_string(),
            });
        }

        assert_eq!(buffer.len(), 101);
        // Nothing was applied here: the buffer is data, the DOM is untouched
        // until `Dom::apply_buffer` runs once.
        assert_eq!(buffer.into_patches().len(), 101);
    }

    /// Ids never collide with the reserved root id.
    #[test]
    fn allocator_reserves_the_root_id() {
        let mut nodes = NodeAllocator::new();
        assert_ne!(nodes.alloc(), NodeId::ROOT);
        assert_eq!(nodes.alloc().index(), 2);
    }

    /// `clear` keeps the frame usable for the next cycle.
    #[test]
    fn clear_empties_the_frame() {
        let mut buffer = PatchBuffer::new();
        buffer.push(Patch::RemoveAttribute {
            node: NodeId::ROOT,
            name: "aria-label".to_owned(),
        });
        buffer.clear();
        assert!(buffer.is_empty());
    }
}
