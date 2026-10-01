//! The column layout of a grid element (plan point 36): the layout itself is
//! portable (`opengrid_grid::layout`, issue #144); here it is kept per host.

pub use opengrid_grid::layout::{ColumnLayout, WIDTH_STEP};

#[cfg(target_arch = "wasm32")]
pub use host::{layout, update};

/// Per-host storage, mirroring the texts and formats seams.
#[cfg(target_arch = "wasm32")]
mod host {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    use opengrid_web_core::host::{id as host_id, on_release};
    use web_sys::HtmlElement;

    use super::ColumnLayout;

    thread_local! {
        static LAYOUTS: RefCell<HashMap<u32, Rc<RefCell<ColumnLayout>>>> =
            RefCell::new(HashMap::new());
    }

    fn release(id: u32) {
        LAYOUTS.with(|map| map.borrow_mut().remove(&id));
    }

    /// The layout of `host`, creating an empty one on first use.
    pub fn layout(host: &HtmlElement) -> Rc<RefCell<ColumnLayout>> {
        on_release(release);
        let id = host_id(host);
        LAYOUTS.with(|map| {
            map.borrow_mut()
                .entry(id)
                .or_insert_with(|| Rc::new(RefCell::new(ColumnLayout::default())))
                .clone()
        })
    }

    /// Changes the layout of `host` and answers whether anything changed.
    pub fn update(host: &HtmlElement, change: impl FnOnce(&mut ColumnLayout) -> bool) -> bool {
        let layout = layout(host);
        let mut layout = layout.borrow_mut();
        change(&mut layout)
    }
}
