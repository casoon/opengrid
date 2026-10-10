//! `opengrid-export` — a query result as CSV or JSON (plan point 83, E33), and on
//! the server as XLSX (feature `xlsx`, issue #72).
//!
//! One implementation of the notation for every place that exports: the
//! server streams through it (point 85), the browser calls it through the
//! element module (point 84). It writes **pieces** — a header, then rows per
//! result — so a million rows never have to be one string.
//!
//! **Raw values, in the notation of the wire form** (E33): a decimal exact as
//! it was stored, a date `YYYY-MM-DD`, a timestamp ISO in UTC with its
//! microseconds, `NaN`/`Infinity`/`-Infinity` spelled out (E13). A display
//! format is the page's; an export is data for the next machine.
//!
//! A pivot is exported **as it is shown** ([`pivot_csv`], issue #3): its
//! values follow the same rules, its headers are the element's words.
//!
//! Portable: no `web-sys`, no `js-sys` (plan/spezifikation/11-crates.md
//! §Portabilität), no Arrow — [`QueryResult`] is Arrow-free (E14).

mod csv;
mod json;
mod pivot;
#[cfg(feature = "xlsx")]
mod xlsx;

pub use csv::{CsvOptions, CsvWriter, csv_header, csv_rows};
pub use json::{JsonWriter, json_rows};
pub use pivot::{PATH_SEPARATOR, PivotLabels, pivot_csv};
#[cfg(feature = "xlsx")]
pub use xlsx::{XLSX_MAX_ROWS, XLSX_MEDIA_TYPE, XlsxWriter};

use std::borrow::Cow;

use opengrid_datasource::{FLAT_COLUMNS, QueryResult};
use opengrid_types::{DataType, Field, FieldName, Schema, Value};

/// How a flat tree's path is written in CSV and XLSX (T8, issue #166): its
/// keys from the root down, `1 / 2 / 4`. JSON writes it as an array.
pub const TREE_PATH_SEPARATOR: &str = " / ";

/// A result with a flat tree's columns after its own (T8, issue #166):
/// `level`, `path` — the keys joined by [`TREE_PATH_SEPARATOR`] — and, when
/// the query filtered, `match`. Anything else is the result as it is.
pub fn with_tree_columns(result: &QueryResult) -> Cow<'_, QueryResult> {
    let Some(flat) = result.tree.as_ref().and_then(|tree| tree.flat.as_ref()) else {
        return Cow::Borrowed(result);
    };
    let name = |text: &str| FieldName::new(text).expect("a flat tree's column is a field name");
    let mut fields = result.schema.fields().to_vec();
    let mut columns = result.columns.clone();
    fields.push(Field::required(name(FLAT_COLUMNS[0]), DataType::Int64));
    columns.push(
        flat.levels
            .iter()
            .map(|level| Value::Int64(i64::try_from(*level).unwrap_or(i64::MAX)))
            .collect(),
    );
    fields.push(Field::required(name(FLAT_COLUMNS[1]), DataType::Utf8));
    columns.push(
        flat.paths
            .iter()
            .map(|path| {
                Value::Utf8(
                    path.iter()
                        .map(|key| plain(key).unwrap_or_default())
                        .collect::<Vec<_>>()
                        .join(TREE_PATH_SEPARATOR),
                )
            })
            .collect(),
    );
    if flat.filtered {
        let matched = result.tree.as_ref().map(|tree| &tree.matched);
        fields.push(Field::required(name(FLAT_COLUMNS[2]), DataType::Bool));
        columns.push(
            matched
                .into_iter()
                .flatten()
                .map(|hit| Value::Bool(*hit))
                .collect(),
        );
    }
    let mut extended = QueryResult::new(Schema::new(fields), columns, result.total_count);
    extended.tree = result.tree.clone();
    Cow::Owned(extended)
}

/// A value as text in the wire notation; `None` for NULL.
///
/// The JSON writer does not use this — it writes the wire form itself — but
/// both follow the same rules, so a CSV cell and a JSON value read the same.
fn plain(value: &Value) -> Option<String> {
    Some(match value {
        Value::Null => return None,
        Value::Bool(flag) => flag.to_string(),
        Value::Int64(number) => number.to_string(),
        // The wire's spelling: the shortest text that reads back to the same
        // f64, and the non-finite ones as the wire names them (E13).
        Value::Float64(_) => match opengrid_json::ToJson::to_json(value) {
            opengrid_json::Json::String(name) => name,
            number => number.to_string(),
        },
        Value::Decimal(decimal) => decimal.to_string(),
        Value::Utf8(text) => text.clone(),
        Value::Date(date) => date.to_string(),
        Value::Timestamp(timestamp) => timestamp.to_string(),
    })
}

#[cfg(test)]
mod tree_tests {
    use super::*;
    use opengrid_datasource::{FlatTree, TreeLevel};

    /// Sales → North → Alice, filtered to Alice: two rows of context, one match.
    fn flat_tree() -> QueryResult {
        let schema = Schema::new(vec![Field::required(
            FieldName::new("name").unwrap(),
            DataType::Utf8,
        )]);
        let mut result = QueryResult::new(
            schema,
            vec![vec![
                Value::Utf8("Sales".to_owned()),
                Value::Utf8("North".to_owned()),
                Value::Utf8("Alice".to_owned()),
            ]],
            3,
        );
        result.tree = Some(TreeLevel {
            children: vec![1, 1, 0],
            matched: vec![false, false, true],
            matches: 1,
            flat: Some(FlatTree {
                levels: vec![1, 2, 3],
                paths: vec![
                    vec![Value::Int64(1)],
                    vec![Value::Int64(1), Value::Int64(2)],
                    vec![Value::Int64(1), Value::Int64(2), Value::Int64(4)],
                ],
                key_type: DataType::Int64,
                filtered: true,
            }),
            ..TreeLevel::default()
        });
        result
    }

    /// CSV: the tree's columns after the query's, the path joined (T8, #166).
    #[test]
    fn a_flat_tree_exports_its_level_path_and_match_as_csv() {
        let mut writer = CsvWriter::new(CsvOptions {
            bom: false,
            ..CsvOptions::default()
        });
        assert_eq!(
            writer.write(&flat_tree()),
            "name,level,path,match\r\nSales,1,1,false\r\nNorth,2,1 / 2,false\r\nAlice,3,1 / 2 / 4,true\r\n"
        );
    }

    /// JSON: the path is an array of keys; without a filter there is no `match`.
    #[test]
    fn a_flat_tree_exports_its_path_as_an_array_in_json() {
        let mut writer = JsonWriter::new();
        let text = writer.write(&flat_tree()) + &writer.finish();
        assert!(
            text.contains(r#"{"name":"Alice","level":3,"path":[1,2,4],"match":true}"#),
            "{text}"
        );

        let mut unfiltered = flat_tree();
        if let Some(flat) = unfiltered.tree.as_mut().and_then(|tree| tree.flat.as_mut()) {
            flat.filtered = false;
        }
        let mut writer = JsonWriter::new();
        let text = writer.write(&unfiltered) + &writer.finish();
        assert!(
            text.contains(r#"{"name":"Sales","level":1,"path":[1]}"#),
            "{text}"
        );
        assert!(!text.contains("match"), "{text}");
    }

    /// A result that is not a flat tree is exported as it is.
    #[test]
    fn a_plain_result_gets_no_tree_columns() {
        let mut plain = flat_tree();
        plain.tree = None;
        assert!(matches!(with_tree_columns(&plain), Cow::Borrowed(_)));
    }
}
