//! The editor of a number column (issue #535).
//!
//! A text field with `inputmode="decimal"` (`"numeric"` for integers), not
//! `type="number"`: what a number field accepts with a comma depends on the
//! browser and its locale, not on the page, and Safari reports an empty value for
//! what it cannot read. The field is read with the page's notation
//! ([`paste::notation_of`]) by the one reader paste uses
//! ([`opengrid_grid::paste::parse_number`]); what it cannot read is **refused
//! under the editor** with the reason — never committed as `NULL`, never rounded.
//! The cell editor, the record form and the bulk panel all take this field.

use super::*;

use opengrid_grid::paste::{NumberProblem, number_text, parse_number};
use opengrid_grid::{Finding, Level};

/// Whether a column of `data_type` is edited as a number.
pub(super) fn is_number(data_type: DataType) -> bool {
    matches!(
        data_type,
        DataType::Int64 | DataType::Float64 | DataType::Decimal { .. }
    )
}

/// Whether `editor` is the number field of a column of `data_type` — not the
/// combobox a lookup or a list of choices makes of it, which holds keys.
pub(super) fn applies(data_type: DataType, editor: Option<&Element>) -> bool {
    is_number(data_type) && !editor.is_some_and(combo::is_combo)
}

/// Types an `<input>` as the number field.
pub(super) fn set_up(input: &Element, data_type: DataType) {
    let _ = input.set_attribute("type", "text");
    let mode = if data_type == DataType::Int64 {
        "numeric"
    } else {
        "decimal"
    };
    let _ = input.set_attribute("inputmode", mode);
}

/// A value's text (`1.1`) as the field opens with it: in the page's notation.
pub(super) fn opening_text(host: &HtmlElement, current: &str) -> String {
    number_text(current, &paste::notation_of(host))
}

/// What the field holds, read in the page's notation — or the reason it cannot
/// be, as a sentence in the page's language.
pub(super) fn read(host: &HtmlElement, data_type: DataType, text: &str) -> Result<Value, String> {
    parse_number(text, data_type, &paste::notation_of(host)).map_err(|problem| {
        let texts = texts(host);
        match problem {
            NumberProblem::NotANumber => texts.number_invalid(text),
            NumberProblem::NotAnInteger => texts.integer_invalid(text),
            NumberProblem::Decimals { scale } => texts.number_decimals(text, scale),
            NumberProblem::TooLarge => texts.number_too_large(text),
        }
    })
}

/// The text a page is told for a field: a number in the wire notation
/// (`1234.5`), whatever notation it was typed in, so a page parses it as it
/// always did. Text that is not one — or a combobox's — is as it stands.
pub(super) fn wire_text(
    host: &HtmlElement,
    data_type: DataType,
    editor: Option<&Element>,
    text: &str,
) -> String {
    if applies(data_type, editor)
        && let Ok(value) = read(host, data_type, text)
    {
        return crate::formats::event_text(&value);
    }
    text.to_owned()
}

/// Says under `editor` why its text is refused, once in the live region, and
/// puts the focus back in it: the editor stays open with what was typed.
pub(super) fn refuse(
    host: &HtmlElement,
    runtime: &Rc<RefCell<GridRuntime>>,
    editor: &Element,
    message: &str,
) {
    let finding = Finding {
        level: Level::Error,
        message: message.to_owned(),
    };
    validate::show(editor, Some(&finding));
    validate::announce(host, runtime, Some(&finding));
    if let Ok(editor) = editor.clone().dyn_into::<HtmlElement>() {
        let _ = editor.focus();
    }
}
