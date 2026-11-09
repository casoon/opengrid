//! The renderer trait and the patch application loop.
//!
//! [`Renderer`] is the four operations the plan names
//! (plan/spezifikation/08-rendering.md §Möglichst wenig Abstraktion):
//! `create_element`, `set_attribute`, `append_child`, `set_text`. The trait is
//! generic over the node type so the patch computation can be exercised on the
//! host with a recording renderer, while the browser uses
//! `WebRenderer` over `web-sys`.
//!
//! [`Dom`] owns the renderer plus the mapping from [`NodeId`] to real nodes and
//! applies a whole [`PatchBuffer`] in one call — the single WASM↔DOM crossing per
//! frame (risk R1).

use std::collections::HashMap;

use crate::patch::{NodeId, Patch, PatchBuffer};

/// The DOM operations the grid needs.
///
/// Implementations: `WebRenderer` (browser, `web-sys`) and the recording
/// renderer in this module's tests, which is what makes the batching logic
/// native-testable.
pub trait Renderer {
    /// The real node this renderer works with.
    type Node: Clone;

    /// Creates an element with the given tag.
    fn create_element(&mut self, tag: &str) -> Self::Node;

    /// Sets an attribute on an element node.
    fn set_attribute(&mut self, node: &Self::Node, name: &str, value: &str);

    /// Removes an attribute from an element node.
    fn remove_attribute(&mut self, node: &Self::Node, name: &str);

    /// Makes the node's text content exactly `text`.
    fn set_text(&mut self, node: &Self::Node, text: &str);

    /// Appends `child` as the last child of `parent`.
    fn append_child(&mut self, parent: &Self::Node, child: &Self::Node);

    /// Tells the renderer which node carries the index `id`, for
    /// [`apply_all`](Self::apply_all). Called for the root and for every node
    /// created one patch at a time.
    fn bind(&mut self, _id: u32, _node: &Self::Node) {}

    /// Applies a whole frame in one go, if the renderer can, and says whether it
    /// did. Nodes the frame created are then asked for with
    /// [`node_at`](Self::node_at).
    fn apply_all(&mut self, _patches: &[Patch]) -> bool {
        false
    }

    /// The node `apply_all` created for `id`.
    fn node_at(&self, _id: u32) -> Option<Self::Node> {
        None
    }
}

/// A rendered tree: a renderer plus the nodes it has created so far.
///
/// The root is bound to [`NodeId::ROOT`]; every other node is created by a
/// [`Patch::CreateElement`]. Applying a buffer in order is enough because patch
/// computation emits parents before children (see [`crate::patch`]).
pub struct Dom<R: Renderer> {
    renderer: R,
    nodes: HashMap<NodeId, R::Node>,
}

impl<R: Renderer> Dom<R> {
    /// Creates a tree whose [`NodeId::ROOT`] is `root`.
    pub fn new(renderer: R, root: R::Node) -> Self {
        let mut nodes = HashMap::new();
        let mut renderer = renderer;
        renderer.bind(NodeId::ROOT.index(), &root);
        nodes.insert(NodeId::ROOT, root);
        Self { renderer, nodes }
    }

    /// Applies one patch.
    ///
    /// Panics if a patch names a node that was never created: patches are
    /// produced by us, in order, so that is a bug, not an input error.
    pub fn apply(&mut self, patch: &Patch) {
        match patch {
            Patch::CreateElement { node, tag } => {
                let created = self.renderer.create_element(tag);
                self.renderer.bind(node.index(), &created);
                self.nodes.insert(*node, created);
            }
            Patch::SetAttribute { node, name, value } => {
                let target = self.node(*node).clone();
                self.renderer.set_attribute(&target, name, value);
            }
            Patch::RemoveAttribute { node, name } => {
                let target = self.node(*node).clone();
                self.renderer.remove_attribute(&target, name);
            }
            Patch::SetText { node, text } => {
                let target = self.node(*node).clone();
                self.renderer.set_text(&target, text);
            }
            Patch::AppendChild { parent, child } => {
                let parent = self.node(*parent).clone();
                let child = self.node(*child).clone();
                self.renderer.append_child(&parent, &child);
            }
        }
    }

    /// Applies a whole frame in one pass.
    pub fn apply_buffer(&mut self, buffer: &PatchBuffer) {
        if self.renderer.apply_all(buffer.patches()) {
            for patch in buffer.patches() {
                if let Patch::CreateElement { node, .. } = patch
                    && let Some(created) = self.renderer.node_at(node.index())
                {
                    self.nodes.insert(*node, created);
                }
            }
            return;
        }
        for patch in buffer.patches() {
            self.apply(patch);
        }
    }

    /// The node bound to `id`, if it has been created yet.
    pub fn node(&self, id: NodeId) -> &R::Node {
        self.nodes
            .get(&id)
            .unwrap_or_else(|| panic!("patch references uncreated node {}", id.index()))
    }

    /// The renderer, for reading.
    pub fn renderer(&self) -> &R {
        &self.renderer
    }

    /// The renderer, for browser-only setup.
    pub fn renderer_mut(&mut self) -> &mut R {
        &mut self.renderer
    }

    /// Consumes the tree and answers its renderer.
    pub fn into_renderer(self) -> R {
        self.renderer
    }
}

/// The kinds of patch in an encoded frame, as the JavaScript that applies it
/// reads them.
mod op {
    pub const CREATE: u32 = 0;
    pub const SET_ATTRIBUTE: u32 = 1;
    pub const REMOVE_ATTRIBUTE: u32 = 2;
    pub const SET_TEXT: u32 = 3;
    pub const APPEND: u32 = 4;
}

/// Encodes a frame for a renderer that applies it in one call: six numbers per
/// patch in `ops`, and every name, value and text appended to the one string
/// `text`.
///
/// The six are the kind, the node's index, and two (offset, length) pairs into
/// `text` — a create has the tag in the first, an attribute its name and value,
/// a removal its name, a text its text; an append has the child's index in the
/// place of the first offset. Offsets and lengths count UTF-16 units, because
/// that is what JavaScript slices the string by. `ops` and `text` are emptied
/// first, so a renderer can keep them between frames.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn encode_frame(patches: &[Patch], ops: &mut Vec<u32>, text: &mut String) {
    ops.clear();
    text.clear();
    let mut end = 0u32;
    let mut piece = |ops: &mut Vec<u32>, cut: &str| {
        let length = utf16_len(cut);
        ops.extend([end, length]);
        text.push_str(cut);
        end += length;
    };
    for patch in patches {
        match patch {
            Patch::CreateElement { node, tag } => {
                ops.extend([op::CREATE, node.index()]);
                piece(ops, tag);
                ops.extend([0, 0]);
            }
            Patch::SetAttribute { node, name, value } => {
                ops.extend([op::SET_ATTRIBUTE, node.index()]);
                piece(ops, name);
                piece(ops, value);
            }
            Patch::RemoveAttribute { node, name } => {
                ops.extend([op::REMOVE_ATTRIBUTE, node.index()]);
                piece(ops, name);
                ops.extend([0, 0]);
            }
            Patch::SetText { node, text } => {
                ops.extend([op::SET_TEXT, node.index()]);
                piece(ops, text);
                ops.extend([0, 0]);
            }
            Patch::AppendChild { parent, child } => {
                ops.extend([op::APPEND, parent.index(), child.index(), 0, 0, 0]);
            }
        }
    }
}

/// The length of `text` in UTF-16 units.
fn utf16_len(text: &str) -> u32 {
    if text.is_ascii() {
        text.len() as u32
    } else {
        text.chars().map(|c| c.len_utf16() as u32).sum()
    }
}

#[cfg(target_arch = "wasm32")]
mod web;
#[cfg(target_arch = "wasm32")]
pub use web::WebRenderer;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::{NodeAllocator, NodeId, Patch, PatchBuffer};

    /// A renderer that only records what it was asked to do; node handles are
    /// plain integers. This is how the batching logic is tested on the host.
    #[derive(Debug, Default, PartialEq, Eq)]
    struct RecordingRenderer {
        calls: Vec<String>,
        next_node: u32,
    }

    impl Renderer for RecordingRenderer {
        type Node = u32;

        fn create_element(&mut self, tag: &str) -> u32 {
            self.calls.push(format!("create {tag}"));
            let node = self.next_node;
            self.next_node += 1;
            node
        }

        fn set_attribute(&mut self, node: &u32, name: &str, value: &str) {
            self.calls.push(format!("set {node} {name}={value}"));
        }

        fn remove_attribute(&mut self, node: &u32, name: &str) {
            self.calls.push(format!("remove {node} {name}"));
        }

        fn set_text(&mut self, node: &u32, text: &str) {
            self.calls.push(format!("text {node}={text}"));
        }

        fn append_child(&mut self, parent: &u32, child: &u32) {
            self.calls.push(format!("append {parent} {child}"));
        }
    }

    /// One frame applies as one pass: the recorder sees every operation but the
    /// driver only calls into it from `apply_buffer`.
    #[test]
    fn applying_a_frame_runs_each_patch_once() {
        let mut nodes = NodeAllocator::new();
        let mut buffer = PatchBuffer::new();
        let table = nodes.alloc();
        let caption = nodes.alloc();
        buffer.push(Patch::CreateElement {
            node: table,
            tag: "table".to_owned(),
        });
        buffer.push(Patch::SetAttribute {
            node: table,
            name: "aria-label".to_owned(),
            value: "Orders".to_owned(),
        });
        buffer.push(Patch::CreateElement {
            node: caption,
            tag: "caption".to_owned(),
        });
        buffer.push(Patch::SetText {
            node: caption,
            text: "Orders".to_owned(),
        });
        buffer.push(Patch::AppendChild {
            parent: table,
            child: caption,
        });
        buffer.push(Patch::AppendChild {
            parent: NodeId::ROOT,
            child: table,
        });

        let mut dom = Dom::new(RecordingRenderer::default(), 99);
        dom.apply_buffer(&buffer);
        let renderer = dom.into_renderer();

        assert_eq!(renderer.calls.len(), buffer.len());
        assert_eq!(renderer.calls[0], "create table");
        assert_eq!(renderer.calls[1], "set 0 aria-label=Orders");
        assert_eq!(renderer.calls.last().unwrap(), "append 99 0");
    }

    /// Removing an attribute is part of the same buffered language.
    #[test]
    fn an_empty_label_removes_the_aria_attribute() {
        let mut nodes = NodeAllocator::new();
        let mut buffer = PatchBuffer::new();
        let table = nodes.alloc();
        buffer.push(Patch::CreateElement {
            node: table,
            tag: "table".to_owned(),
        });
        buffer.push(Patch::RemoveAttribute {
            node: table,
            name: "aria-label".to_owned(),
        });

        let mut dom = Dom::new(RecordingRenderer::default(), 0);
        dom.apply_buffer(&buffer);
        assert_eq!(
            dom.into_renderer().calls,
            ["create table", "remove 0 aria-label"]
        );
    }

    /// The pieces of an encoded frame, cut from its string the way the
    /// JavaScript does: by UTF-16 units. A character outside the BMP is two of
    /// them, and a name or value after one would be cut wrongly by its byte or
    /// code point offset.
    fn cut(text: &str, offset: u32, length: u32) -> String {
        let units: Vec<u16> = text.encode_utf16().collect();
        String::from_utf16(&units[offset as usize..(offset + length) as usize]).unwrap()
    }

    #[test]
    fn a_frame_is_encoded_as_six_numbers_per_patch_and_one_string() {
        let mut nodes = NodeAllocator::new();
        let cell = nodes.alloc();
        let row = nodes.alloc();
        let patches = [
            Patch::CreateElement {
                node: cell,
                tag: "td".to_owned(),
            },
            Patch::SetAttribute {
                node: cell,
                name: "data-row".to_owned(),
                value: "Köln 🙂 ü".to_owned(),
            },
            Patch::SetText {
                node: cell,
                text: "🙂🙂 €".to_owned(),
            },
            Patch::RemoveAttribute {
                node: cell,
                name: "aria-invalid".to_owned(),
            },
            Patch::AppendChild {
                parent: row,
                child: cell,
            },
        ];
        let (mut ops, mut text) = (Vec::new(), String::new());
        encode_frame(&patches, &mut ops, &mut text);

        assert_eq!(ops.len(), 6 * patches.len());
        let at = |patch: usize| &ops[patch * 6..patch * 6 + 6];
        assert_eq!(at(0)[..2], [op::CREATE, cell.index()]);
        assert_eq!(cut(&text, at(0)[2], at(0)[3]), "td");
        assert_eq!(at(1)[..2], [op::SET_ATTRIBUTE, cell.index()]);
        assert_eq!(cut(&text, at(1)[2], at(1)[3]), "data-row");
        assert_eq!(cut(&text, at(1)[4], at(1)[5]), "Köln 🙂 ü");
        assert_eq!(at(2)[..2], [op::SET_TEXT, cell.index()]);
        assert_eq!(cut(&text, at(2)[2], at(2)[3]), "🙂🙂 €");
        assert_eq!(at(3)[..2], [op::REMOVE_ATTRIBUTE, cell.index()]);
        assert_eq!(cut(&text, at(3)[2], at(3)[3]), "aria-invalid");
        assert_eq!(at(4), [op::APPEND, row.index(), cell.index(), 0, 0, 0]);
    }

    #[test]
    fn encoding_a_frame_forgets_the_last_one() {
        let mut nodes = NodeAllocator::new();
        let node = nodes.alloc();
        let patch = [Patch::SetText {
            node,
            text: "x".to_owned(),
        }];
        let (mut ops, mut text) = (Vec::new(), String::new());
        encode_frame(&patch, &mut ops, &mut text);
        encode_frame(&patch, &mut ops, &mut text);
        assert_eq!((ops.len(), text.as_str()), (6, "x"));
    }
}
