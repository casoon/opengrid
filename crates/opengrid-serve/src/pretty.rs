//! JSON the owner reads and edits: a value that fits a line stays on it, a
//! longer object or array puts one member per line, two-space indentation.

use opengrid_json::Json;

/// `json` as indented text, ending in a newline.
pub fn pretty(json: &Json) -> String {
    let mut out = String::new();
    write(json, 0, &mut out);
    out.push('\n');
    out
}

const WIDTH: usize = 80;

fn write(json: &Json, depth: usize, out: &mut String) {
    let compact = json.to_string();
    if compact.len() + depth * 2 <= WIDTH {
        return out.push_str(&compact);
    }
    let indent = |out: &mut String, depth: usize| out.push_str(&"  ".repeat(depth));
    match json {
        Json::Array(items) if !items.is_empty() => {
            out.push_str("[\n");
            for (index, item) in items.iter().enumerate() {
                indent(out, depth + 1);
                write(item, depth + 1, out);
                out.push_str(if index + 1 < items.len() { ",\n" } else { "\n" });
            }
            indent(out, depth);
            out.push(']');
        }
        Json::Object(object) if !object.is_empty() => {
            out.push_str("{\n");
            let last = object.len() - 1;
            for (index, (key, value)) in object.iter().enumerate() {
                indent(out, depth + 1);
                out.push_str(&Json::from(key.as_str()).to_string());
                out.push_str(": ");
                write(value, depth + 1, out);
                out.push_str(if index < last { ",\n" } else { "\n" });
            }
            indent(out, depth);
            out.push('}');
        }
        _ => out.push_str(&compact),
    }
}
