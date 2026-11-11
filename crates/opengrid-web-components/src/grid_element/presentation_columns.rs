//! Column presentation: the page's columns — titles, formats, markers — drawn into the grid and read back when they change.

use super::*;

/// Checks the stored `set_columns` configuration against the current schema and
/// rebuilds if it holds.
///
/// Called twice for the same configuration, and deliberately: once when the page
/// sets it, and once when the first result brings the real schema. A page that
/// configures before attaching a provider — which is the ordinary order, exactly
/// as `set_texts` documents — would otherwise be checked against the display
/// schema of point 23, where every column is `Utf8` and `sum` on a number would
/// be refused for being a sum on text.
pub(super) fn recolumn(host: &HtmlElement) {
    recolumn_from(host, None);
}

/// [`crate::element::set_columns`] for a grid: takes the configuration and
/// checks it. Where it pins is read at once (issue #316), so the columns may
/// stand in another order afterwards — the grid is then built again around the
/// view it had.
pub(crate) fn set_columns(host: &HtmlElement, value: &JsValue) {
    let before = arrangement(host);
    presentation::set_columns_for(host, value);
    recolumn_from(host, before);
}

/// [`recolumn`], with the columns and the view as they were `before` the
/// configuration changed.
fn recolumn_from(host: &HtmlElement, mut before: Option<Arrangement>) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    let raw = presentation::raw(host);
    if raw.is_empty() {
        // Nothing to check; a configuration taken away takes its pins along.
        if let Some(before) = before {
            rearrange(host, &runtime, before, None);
        }
        return;
    }
    let schema = runtime.borrow().state.schema().clone();
    let checked = presentation::validate(
        &raw,
        &schema,
        &grid::parse_columns(host.get_attribute(COLUMNS_ATTRIBUTE).as_deref()),
    );
    // A configuration refused against the real schema is applied not at all,
    // its pins included. Before a result has typed a column the refusal may be
    // the display schema's (see above), and the pins stay as asked for.
    let refused = checked.is_err() && !runtime.borrow().known.is_empty();
    if presentation::is_refused(host) != refused {
        before = before.or_else(|| arrangement(host));
        presentation::refuse(host, refused);
    }
    match checked {
        Ok(checked) => {
            let styles = presentation::ColumnStyles::new(checked);
            let same = *presentation::styles(host) == styles;
            if !same {
                presentation::store(host, styles);
            }
            if before.is_some_and(|before| rearrange(host, &runtime, before, None)) || same {
                return;
            }
            // The markers live in the one-time skeleton (a pool cell always
            // shows the same column), so a new presentation is a new skeleton
            // — around the same state: the columns are the same, and a view
            // applied before the first result must not be thrown away with the
            // old skeleton (point 88). The aggregates and facets the
            // presentation names are asked for again, as the rebuild did.
            {
                let mut borrowed = runtime.borrow_mut();
                if let Some(grouping) = borrowed.grouping.as_mut() {
                    grouping.invalidate();
                }
                borrowed.groups_filter = None;
            }
            let had_focus = reskeleton(host, &runtime);
            run_query(host, QueryKind::Data, had_focus);
        }
        Err(problems) => {
            if let Some(before) = before {
                rearrange(host, &runtime, before, None);
            }
            let message = problems
                .iter()
                .map(|problem| format!("{}: {}", problem.column, problem.reason))
                .collect::<Vec<_>>()
                .join(" ");
            runtime
                .borrow_mut()
                .state
                .set_status(GridStatus::Error(message));
            render(host, false);
        }
    }
}
