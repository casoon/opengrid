//! The filter row: reading it into entries, applying them, what a change of an operator or a value does, and the Clear button.

use super::*;

/// Whether `element` sits inside the filter row (and not in the grid table).
pub(super) fn is_filter_control(element: &Element) -> bool {
    element.closest("[data-filter]").ok().flatten().is_some()
}

/// Reads the filter row into [`FilterEntry`]s, one per output column.
///
/// A column's value control in the filter row: the text input, or for a
/// boolean column the choice of *any*, *yes*, *no* (issue #60). Every read
/// and write of a filter value goes through here, so the two cannot drift.
pub(super) enum ValueControl {
    Input(HtmlInputElement),
    Choice(HtmlSelectElement),
}

impl ValueControl {
    /// The control that is shown for column `col`.
    pub(super) fn of(root: &ShadowRoot, col: usize) -> Option<Self> {
        if let Ok(Some(node)) =
            root.query_selector(&format!("select[data-value-col=\"{col}\"]:not([hidden])"))
            && let Ok(choice) = node.dyn_into::<HtmlSelectElement>()
        {
            return Some(Self::Choice(choice));
        }
        root.query_selector(&format!("input[data-col=\"{col}\"]"))
            .ok()
            .flatten()
            .and_then(|node| node.dyn_into::<HtmlInputElement>().ok())
            .map(Self::Input)
    }

    pub(super) fn value(&self) -> String {
        match self {
            Self::Input(input) => input.value(),
            Self::Choice(choice) => choice.value(),
        }
    }

    pub(super) fn set_value(&self, value: &str) {
        match self {
            Self::Input(input) => input.set_value(value),
            Self::Choice(choice) => {
                choice.set_value(value);
                // A key the lookup does not name — a view saved before the
                // page's map changed — is still the filter: it gets an option
                // of its own, named by the key as a cell names it
                // (issue #326). A boolean's three options are always there.
                if choice.value() != value
                    && let Some(option) = choice
                        .owner_document()
                        .and_then(|document| document.create_element("option").ok())
                {
                    let _ = option.set_attribute("value", value);
                    option.set_text_content(Some(value));
                    let _ = choice.append_child(&option);
                    choice.set_value(value);
                }
            }
        }
    }

    pub(super) fn set_disabled(&self, disabled: bool) {
        match self {
            Self::Input(input) => input.set_disabled(disabled),
            Self::Choice(choice) => choice.set_disabled(disabled),
        }
    }

    pub(super) fn is_disabled(&self) -> bool {
        match self {
            Self::Input(input) => input.disabled(),
            Self::Choice(choice) => choice.disabled(),
        }
    }

    pub(super) fn element(&self) -> &HtmlElement {
        match self {
            Self::Input(input) => input,
            Self::Choice(choice) => choice,
        }
    }
}

/// A column's operator in the filter row: the button inside its field
/// (issue #96). Every read and write of a filter's comparison goes through
/// here, like the value through [`ValueControl`].
pub(super) struct OperatorControl {
    button: HtmlElement,
}

impl OperatorControl {
    pub(super) fn of(root: &ShadowRoot, col: usize) -> Option<Self> {
        root.query_selector(&format!("[part=\"filter-operator\"][data-col=\"{col}\"]"))
            .ok()
            .flatten()
            .and_then(|node| node.dyn_into::<HtmlElement>().ok())
            .map(|button| Self { button })
    }

    /// The comparisons the column's type allows, in the menu's order.
    pub(super) fn allowed(&self) -> Vec<String> {
        self.button
            .get_attribute("data-ops")
            .map(|ops| ops.split_whitespace().map(str::to_owned).collect())
            .unwrap_or_default()
    }

    /// The chosen comparison; before one is chosen, the type's first.
    pub(super) fn value(&self) -> String {
        self.button
            .get_attribute("data-op")
            .filter(|op| !op.is_empty())
            .or_else(|| self.allowed().into_iter().next())
            .unwrap_or_default()
    }

    /// Chooses `op`, and says so: the sign on the button, the words in its
    /// name.
    pub(super) fn set_value(&self, op: &str) {
        let _ = self.button.set_attribute("data-op", op);
        self.button.set_text_content(Some(grid::operator_glyph(op)));
        let Some(host) = self
            .button
            .get_root_node()
            .dyn_into::<ShadowRoot>()
            .ok()
            .and_then(|root| root.host().dyn_into::<HtmlElement>().ok())
        else {
            return;
        };
        let texts = texts(&host);
        let column = self
            .button
            .get_attribute("data-col")
            .and_then(|col| col.parse::<usize>().ok())
            .and_then(|col| columns_of(&host).get(col).cloned())
            .unwrap_or_default();
        let word = operator_word(&texts, op);
        let _ = self.button.set_attribute(
            "aria-label",
            &format!("{}: {}", texts.operator_label(&column), word),
        );
        // The sign alone is terse: the empty field says the comparison in
        // words too. Not a label — the field has its name — a hint.
        if let Some(col) = self.button.get_attribute("data-col")
            && let Some(root) = host.shadow_root()
            && let Ok(Some(input)) = root.query_selector(&format!("input[data-col=\"{col}\"]"))
        {
            let _ = input.set_attribute("placeholder", &word);
        }
    }

    /// Back to the type's default: the first comparison it allows, or — while
    /// the type is not known yet — none chosen, so the first result picks it.
    pub(super) fn reset(&self) {
        match self.allowed().into_iter().next() {
            Some(first) => self.set_value(&first),
            None => {
                let _ = self.button.remove_attribute("data-op");
            }
        }
    }

    pub(super) fn element(&self) -> &HtmlElement {
        &self.button
    }
}

/// The word for `op`, as the menu and the button's name say it.
pub(super) fn operator_word(texts: &crate::texts::GridTexts, op: &str) -> String {
    let index = crate::shared::FILTER_OPERATORS
        .iter()
        .position(|candidate| *candidate == op)
        .unwrap_or(0);
    texts.operator(index, op)
}

/// The operator `select` and value `input` are ordinary form controls; a missing
/// control (should not happen after the skeleton) falls back to `eq`/empty.
pub(super) fn read_filter_entries(root: &ShadowRoot, columns: &[String]) -> Vec<FilterEntry> {
    columns
        .iter()
        .enumerate()
        .map(|(col, column)| {
            let op = OperatorControl::of(root, col)
                .map(|operator| operator.value())
                .unwrap_or_default();
            let value = ValueControl::of(root, col)
                .map(|control| control.value())
                .unwrap_or_default();
            FilterEntry {
                column: column.clone(),
                op: grid::FilterOp::parse(&op).unwrap_or(grid::FilterOp::Cmp(CmpOp::Eq)),
                value,
            }
        })
        .collect()
}

/// Applies the filter row: builds the `filter` expression, returns to the first
/// window and re-runs the query without moving focus (it stays in the control
/// the user typed into).
///
/// An input the column cannot hold does **not** become a query (point 51). It
/// lands in the status line instead, which is announced — the user typed it, so
/// the user hears about it, rather than the engine answering with a validation
/// error about a value nobody can see.
pub(super) fn apply_filters(host: &HtmlElement) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    let Some(runtime) = runtime(host) else {
        return;
    };
    let columns = columns_of(host);
    let entries = read_filter_entries(&root, &columns);
    let schema = runtime.borrow().state.schema().clone();

    let filter = match grid::filter_expr(&entries, &schema) {
        Ok(filter) => filter,
        Err(problems) => {
            let texts = texts(host);
            let message = problems
                .iter()
                .map(|problem| texts.filter_invalid(&problem.column, &problem.value))
                .collect::<Vec<_>>()
                .join(" ");
            runtime
                .borrow_mut()
                .state
                .set_status(GridStatus::Error(message));
            render(host, false);
            return;
        }
    };

    let pool = pool_of(host);
    {
        let mut runtime = runtime.borrow_mut();
        runtime.state.set_filter(filter);
        runtime.state.set_window(Window::new(0, pool));
    }
    // A filter changes which rows exist, so the selection is gone (point 35).
    announce_if_cleared(host, &runtime);
    run_query(host, QueryKind::Data, false);
    dispatch_view(host);
}

/// An operator control changed: bring its value field in line.
pub(super) fn on_filter_change(event: Event) {
    if let Some(target) = event
        .target()
        .and_then(|node| node.dyn_into::<Element>().ok())
        && target.closest("[part=\"facets\"]").ok().flatten().is_some()
        && let Some(root) = current_shadow_root(&event)
        && let Ok(host) = root.host().dyn_into::<HtmlElement>()
    {
        on_facet_change(&host, &target);
        return;
    }
    let Some(root) = current_shadow_root(&event) else {
        return;
    };
    let Ok(host) = root.host().dyn_into::<HtmlElement>() else {
        return;
    };
    // A column-visibility checkbox (point 36): ordinary form behaviour, so the
    // keyboard needs nothing special.
    if let Some(target) = event
        .target()
        .and_then(|node| node.dyn_into::<Element>().ok())
        && let Some(name) = target.get_attribute("data-column")
        && let Ok(input) = target.dyn_into::<HtmlInputElement>()
        && input.type_() == "checkbox"
    {
        set_column_hidden(&host, &name, !input.checked());
        return;
    }
    let Some(runtime) = runtime(&host) else {
        return;
    };
    let schema = runtime.borrow().state.schema().clone();
    fix_operator_choices(&root, &schema, &presentation::styles(&host));
}

/// What [`fix_operator_choices`] and [`fix_presentation`] work from: the
/// columns' types and the markers the page's configuration gives them.
#[derive(PartialEq)]
pub(super) struct Decorated {
    schema: opengrid_types::Schema,
    markers: Vec<Vec<(&'static str, String)>>,
}

impl Decorated {
    pub(super) fn of(schema: &opengrid_types::Schema, styles: &presentation::ColumnStyles) -> Self {
        Self {
            schema: schema.clone(),
            markers: schema
                .fields()
                .iter()
                .map(|field| styles.markers(field.name.as_str(), field.data_type))
                .collect(),
        }
    }
}

/// Moves a column's operator control onto an operator its type allows.
///
/// The options are built before the types are known (point 51), so the default
/// selection can be one the column does not offer — `contains` on a number.
/// Patches can disable that option, but the *selection* is a DOM property, so it
/// is corrected here, after the frame landed.
/// Writes the presentation markers of point 60 onto the header and the pool.
///
/// The same reason `fix_operator_choices` exists next to it: the skeleton is
/// built at connect, from the **display schema** of point 23 where every column
/// is `Utf8` — and the real types only arrive with the first result. An
/// alignment derived at build time would say "text, so left" about every
/// column, including the numbers, and would stay wrong for the life of the
/// grid.
///
/// Written per result rather than per frame: [`render`] calls it only when the
/// [`Decorated`] it would write differs from the last one.
pub(super) fn fix_presentation(
    root: &ShadowRoot,
    schema: &opengrid_types::Schema,
    styles: &presentation::ColumnStyles,
) {
    for (col, field) in schema.fields().iter().enumerate() {
        let markers = styles.markers(field.name.as_str(), field.data_type);
        let Ok(cells) = root.query_selector_all(&format!("[data-col=\"{col}\"]")) else {
            continue;
        };
        for index in 0..cells.length() {
            let Some(cell) = cells
                .item(index)
                .and_then(|node| node.dyn_into::<Element>().ok())
            else {
                continue;
            };
            // Only the table's own cells: the filter row shares `data-col`.
            if !matches!(cell.tag_name().as_str(), "TD" | "TH") {
                continue;
            }
            for name in ["data-mono", "data-emphasis", "data-muted", "aria-readonly"] {
                if !markers.iter().any(|(marker, _)| *marker == name) {
                    let _ = cell.remove_attribute(name);
                }
            }
            for (name, value) in &markers {
                let _ = cell.set_attribute(name, value);
            }
        }
    }
}

pub(super) fn fix_operator_choices(
    root: &ShadowRoot,
    schema: &opengrid_types::Schema,
    styles: &presentation::ColumnStyles,
) {
    for (col, field) in schema.fields().iter().enumerate() {
        let Some(operator) = OperatorControl::of(root, col) else {
            continue;
        };
        let allowed = grid::operators_of(
            field.data_type,
            field.nullable,
            styles.lookup(field.name.as_str()).is_some(),
        );
        let chosen = operator
            .element()
            .get_attribute("data-op")
            .filter(|op| allowed.contains(&op.as_str()));
        // Written every time, not only on a change: the button's sign and name
        // come from here, and the first result is when its type is known.
        let op = chosen.unwrap_or_else(|| allowed.first().copied().unwrap_or("eq").to_owned());
        operator.set_value(&op);

        // The two operators that take no value say so: their input is disabled
        // rather than silently ignored.
        if let Some(control) = ValueControl::of(root, col) {
            control.set_disabled(!grid::takes_value(&op));
        }
    }
}

/// Clears every value input and applies the (now empty) filter.
pub(super) fn on_filter_clear(event: Event) {
    let Some(root) = current_shadow_root(&event) else {
        return;
    };
    let Some(target) = event
        .target()
        .and_then(|node| node.dyn_into::<Element>().ok())
    else {
        return;
    };
    // The column list opens and closes from its own button (point 36), as a
    // panel under it (issue #101).
    if let Ok(Some(button)) = target.closest("button[part=\"columns-toggle\"]")
        && let Some(root) = current_shadow_root(&event)
    {
        if button.get_attribute("aria-expanded").as_deref() == Some("true") {
            close_columns(&root, false);
        } else {
            open_columns(&root, true);
        }
        return;
    }

    // An option or a chip of a combobox (issue #185).
    if let Ok(host) = root.host().dyn_into::<HtmlElement>()
        && combo::on_click(&host, &target)
    {
        return;
    }

    // The buttons of the bulk panel (issue #187).
    if let Ok(host) = root.host().dyn_into::<HtmlElement>()
        && bulk::on_click(&host, &target)
    {
        return;
    }

    // The buttons of a paste preview (issue #184).
    if let Ok(host) = root.host().dyn_into::<HtmlElement>()
        && paste::on_preview_click(&host, &target)
    {
        return;
    }

    // The "Refresh" button of a changed source (issue #328).
    if let Ok(host) = root.host().dyn_into::<HtmlElement>()
        && live::is_button(&target)
    {
        live::refresh(&host);
        return;
    }

    // The button of a row's detail panel (issue #318).
    if let Ok(host) = root.host().dyn_into::<HtmlElement>()
        && detail::on_click(&host, &target)
    {
        return;
    }

    // An action button of a row (issue #189).
    if let Ok(host) = root.host().dyn_into::<HtmlElement>()
        && on_action_click(&host, &target)
    {
        return;
    }

    // The same listener serves the paging buttons (point 38); keyboard
    // activation fires a click too, so `Enter`/`Space` on them works natively.
    if let Ok(host) = root.host().dyn_into::<HtmlElement>()
        && on_pager_click(&host, &target)
    {
        return;
    }

    // The empty state's reset (point 68): the same as "Remove all" (65), not
    // a second way with its own behaviour.
    if target
        .closest("[data-empty-reset]")
        .ok()
        .flatten()
        .is_some()
        && let Ok(host) = root.host().dyn_into::<HtmlElement>()
    {
        clear_chips(&host);
        // The button is gone with the rows back; the focus goes to the top of
        // the grid rather than to the document.
        if let Some(runtime) = runtime(&host)
            && root
                .query_selector("[data-toolbar=\"filter-row\"]")
                .ok()
                .flatten()
                .is_none()
        {
            runtime
                .borrow_mut()
                .set_active(ActiveCell::Header { col: 0 });
            focus_active(&host);
        }
        return;
    }

    // A suggestion of the search field (point 67).
    if let Ok(Some(option)) = target.closest("[part=\"search-list\"] [role=\"option\"]")
        && let Ok(host) = root.host().dyn_into::<HtmlElement>()
    {
        take_suggestion(&host, &option);
        return;
    }

    // The facet sidebar (point 66): its switch, its pills, its reset.
    if let Ok(host) = root.host().dyn_into::<HtmlElement>() {
        if target
            .closest("[data-toolbar=\"facets\"]")
            .ok()
            .flatten()
            .is_some()
        {
            let _ = if host.has_attribute(grid::FACETS_ATTRIBUTE) {
                host.remove_attribute(grid::FACETS_ATTRIBUTE)
            } else {
                host.set_attribute(grid::FACETS_ATTRIBUTE, "")
            };
            return;
        }
        if let Ok(Some(pill)) = target.closest("[part=\"facet-pill\"]") {
            toggle_facet_value(&host, &pill);
            return;
        }
        if target
            .closest("[data-facets-reset]")
            .ok()
            .flatten()
            .is_some()
        {
            reset_facets(&host);
            return;
        }
    }

    // The filter row's operator button and its menu (issue #96).
    if let Ok(host) = root.host().dyn_into::<HtmlElement>() {
        if let Ok(Some(item)) = target.closest("[part=\"operator-menu\"] [role=\"menuitemradio\"]")
        {
            pick_operator(&host, &item);
            return;
        }
        if let Ok(Some(button)) = target.closest("[part=\"filter-operator\"]")
            && let Some(col) = button
                .get_attribute("data-col")
                .and_then(|col| col.parse::<usize>().ok())
        {
            if button.get_attribute("aria-expanded").as_deref() == Some("true") {
                close_operator_menu(&host, true);
            } else {
                open_operator_menu(&host, col, false);
            }
            return;
        }
    }

    // The quick doors of issue #34: their buttons, the dialog's, the menu's.
    if let Ok(host) = root.host().dyn_into::<HtmlElement>() {
        if target
            .closest("[data-toolbar=\"add-filter\"]")
            .ok()
            .flatten()
            .is_some()
        {
            open_filter_dialog(&host);
            return;
        }
        // The builder of the nested filter (issue #324), from the toolbar
        // and from the filter's chip.
        let chip = target.closest("[data-chip-edit]").ok().flatten().is_some();
        if chip
            || target
                .closest("[data-toolbar=\"filter-builder\"]")
                .ok()
                .flatten()
                .is_some()
        {
            builder::open(&host, chip);
            return;
        }
        if let Ok(Some(button)) = target.closest("[data-toolbar=\"add-grouping\"]") {
            if button.get_attribute("aria-disabled").as_deref() != Some("true") {
                open_group_menu(&host);
            }
            return;
        }
        if let Ok(Some(action)) = target.closest("[data-dialog-action]") {
            match action.get_attribute("data-dialog-action").as_deref() {
                Some("apply") => apply_filter_dialog(&host),
                _ => close_quick(&host, "filter-dialog", "add-filter", true),
            }
            return;
        }
        if let Ok(Some(item)) = target.closest("[part=\"grouping-menu\"] [role=\"menuitem\"]") {
            pick_grouping(&host, &item);
            return;
        }
    }

    // The toolbar and the chips (point 65).
    if let Ok(host) = root.host().dyn_into::<HtmlElement>() {
        if target
            .closest("[data-toolbar=\"filter-row\"]")
            .ok()
            .flatten()
            .is_some()
        {
            toggle_filter_row(&host);
            return;
        }
        if let Ok(Some(button)) = target.closest("[data-density]")
            && let Some(density) = button.get_attribute("data-density")
        {
            let _ = host.set_attribute(grid::DENSITY_ATTRIBUTE, &density);
            return;
        }
        if let Ok(Some(button)) = target.closest("[data-chip-remove]") {
            remove_chip(&host, &button);
            return;
        }
        if target
            .closest("[data-chips-clear]")
            .ok()
            .flatten()
            .is_some()
        {
            clear_chips(&host);
            return;
        }
    }

    // The column menu (point 64): its trigger opens it, its entries act.
    if let Ok(host) = root.host().dyn_into::<HtmlElement>() {
        if let Ok(Some(item)) = target.closest("[part=\"column-menu\"] [data-action]") {
            activate_menu_item(&host, &item);
            return;
        }
        if let Ok(Some(trigger)) = target.closest("[part=\"column-menu-button\"]")
            && let Some(col) = trigger
                .closest("th")
                .ok()
                .flatten()
                .and_then(|th| th.get_attribute("data-col"))
                .and_then(|col| col.parse::<usize>().ok())
        {
            open_column_menu(&host, col);
            return;
        }
    }

    // A group row (point 62): a click anywhere on it opens or closes it — the
    // pointer path to what `Enter` does.
    if let Ok(Some(row)) = target.closest("tr[data-kind=\"group\"]")
        && let Ok(host) = root.host().dyn_into::<HtmlElement>()
        && let Some(position) = row
            .query_selector("td[data-row]")
            .ok()
            .flatten()
            .and_then(|cell| cell.get_attribute("data-row"))
            .and_then(|row| row.parse::<u64>().ok())
    {
        toggle_group(&host, position);
        return;
    }

    // A tree's node (issue #135): a click on its first cell — where the
    // chevron is — opens or closes it; a leaf has nothing to open.
    if let Ok(Some(cell)) = target.closest("tr[data-tree][aria-expanded] td[data-col=\"0\"]")
        && let Ok(host) = root.host().dyn_into::<HtmlElement>()
        && let Some(position) = cell
            .get_attribute("data-row")
            .and_then(|row| row.parse::<u64>().ok())
    {
        toggle_tree(&host, position);
        return;
    }

    // The selection column (point 61). The mouse path only: the keys already
    // reach it through the grid matrix, and a `click` here would otherwise fire
    // a second time for the keyboard activation.
    if let Ok(Some(cell)) = target.closest("[data-select]")
        && let Ok(host) = root.host().dyn_into::<HtmlElement>()
        && let Some(runtime) = runtime(&host)
    {
        // Grouped, both select by key or — without a `row-key` — not at all.
        match active_from_element(&cell) {
            Some(ActiveCell::SelectAll) => {
                toggle_all(&host, &runtime);
                return;
            }
            Some(ActiveCell::Select { row }) => {
                toggle_row(&host, &runtime, row, false);
                return;
            }
            _ => {}
        }
    }
    if target
        .closest("[data-filter-clear]")
        .ok()
        .flatten()
        .is_none()
    {
        return;
    }
    let Ok(host) = root.host().dyn_into::<HtmlElement>() else {
        return;
    };
    if let Ok(inputs) = root.query_selector_all("input[data-col]") {
        for index in 0..inputs.length() {
            if let Some(node) = inputs.item(index)
                && let Ok(input) = node.dyn_into::<HtmlInputElement>()
            {
                input.set_value("");
            }
        }
    }
    if let Ok(choices) = root.query_selector_all("select[data-value-col]") {
        for index in 0..choices.length() {
            if let Some(node) = choices.item(index)
                && let Ok(choice) = node.dyn_into::<HtmlSelectElement>()
            {
                choice.set_value("");
            }
        }
    }
    apply_filters(&host);
}
