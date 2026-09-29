//! The writer: compact, in the spelling `serde_json` used.

use crate::{Json, Number};

pub(crate) fn write(json: &Json, out: &mut String) {
    match json {
        Json::Null => out.push_str("null"),
        Json::Bool(true) => out.push_str("true"),
        Json::Bool(false) => out.push_str("false"),
        Json::Number(Number::Float(value)) => out.push_str(&write_f64(*value)),
        Json::Number(number) => out.push_str(&number.to_string()),
        Json::String(text) => string(text, out),
        Json::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write(item, out);
            }
            out.push(']');
        }
        Json::Object(object) => {
            out.push('{');
            for (index, (key, value)) in object.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                string(key, out);
                out.push(':');
                write(value, out);
            }
            out.push('}');
        }
    }
}

/// A string with the escapes `serde_json` wrote: the quote, the backslash,
/// the five short control escapes, `\u00XX` (lower-case) for the other
/// control characters, everything else as it is.
fn string(text: &str, out: &mut String) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            control if (control as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", control as u32));
            }
            other => out.push(other),
        }
    }
    out.push('"');
}

/// A finite float in the layout `serde_json` wrote: the shortest digits that
/// read back to the same number; plain notation from `0.00001` to below `1e16`
/// (`1.0`, `0.5`, `-0.0`), else scientific with an explicit sign on a positive
/// exponent (`1e+16`, `1.5e-7`).
///
/// # Panics
/// On NaN or an infinity, which JSON cannot spell; the callers write those
/// as `null` or as strings.
pub fn write_f64(value: f64) -> String {
    assert!(value.is_finite(), "{value} has no JSON spelling");
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0.0"
        } else {
            "0.0"
        }
        .to_owned();
    }
    // `{:e}` gives the shortest round-trip digits: `d.ddde±X`.
    let scientific = format!("{value:e}");
    let (negative, scientific) = match scientific.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, scientific.as_str()),
    };
    let (mantissa, exponent) = scientific.split_once('e').expect("`{:e}` has an exponent");
    let exponent: i32 = exponent.parse().expect("`{:e}` writes an integer exponent");
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let count = digits.len() as i32;
    // The value is `0.digits × 10^point`.
    let point = exponent + 1;

    let mut out = String::with_capacity(count as usize + 8);
    if negative {
        out.push('-');
    }
    if count <= point && point <= 16 {
        out.push_str(&digits);
        out.extend(std::iter::repeat_n('0', (point - count) as usize));
        out.push_str(".0");
    } else if 0 < point && point <= 16 {
        out.push_str(&digits[..point as usize]);
        out.push('.');
        out.push_str(&digits[point as usize..]);
    } else if -5 < point && point <= 0 {
        out.push_str("0.");
        out.extend(std::iter::repeat_n('0', (-point) as usize));
        out.push_str(&digits);
    } else {
        out.push_str(&digits[..1]);
        if count > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        out.push('e');
        if exponent > 0 {
            out.push('+');
        }
        out.push_str(&exponent.to_string());
    }
    out
}
