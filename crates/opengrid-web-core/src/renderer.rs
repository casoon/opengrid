//! The renderer trait and the patch application loop.
//!
//! [`Renderer`] is the four operations the plan names
//! (plan/spezifikation/08-rendering.md §Möglichst wenig Abstraktion):
//! `create_element`, `set_attribute`, `append_child`, `set_text`. The trait is
//! generic over the node type so the patch computation can be exercised on the
//! host with a recording renderer, while the browser uses
//! [`WebRenderer`] over `web-sys`.
//!
//! [`Dom`] owns the renderer plus the mapping from [`NodeId`] to real nodes and
//! applies a whole [`PatchBuffer`] in one call — the single WASM↔DOM crossing per
//! frame (risk R1).

use std::collections::HashMap;

use crate::patch::{NodeId, Patch, PatchBuffer};

/// The DOM operations the grid needs.
///
/// Implementations: [`WebRenderer`] (browser, `web-sys`) and the recording
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
}
