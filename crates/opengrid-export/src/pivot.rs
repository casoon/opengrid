//! A pivot as CSV, the way `<opengrid-pivot>` shows it (issue #3).
//!
//! The row dimensions are columns, then one column per generated column; every
//! row the element shows is a line — the subtotals and the grand total
//! included, each with its label. Values follow the CSV rules of this crate
//! (wire notation, NULL, the formula guard); only the **headers** are words,
//! and they are the element's words, passed in as [`PivotLabels`].
//!
//! # One header line, composed
//!
//! The element heads a column dimension with two rows: the value spanning its
//! measures, the measures below. A CSV gets **one** line, each name composed
//! from the path and the measure — `2025 · total`:
//!
//! - Every reader of a CSV — a spreadsheet's filter, pandas, a database's
//!   `COPY`, opengrid's own ingest — takes the first line as the names and the
//!   second as data. A second header line arrives as a row of text in the
//!   number columns, and each reader then needs to be told otherwise.
//! - A CSV has no merged cells. A two-line header would either repeat the value
//!   over each of its measures — the composed name, split in two — or leave
//!   cells empty, and an empty header cell is the silence the element refuses;
//!   it would also lose its value the moment a column is moved or hidden in the
//!   spreadsheet.
//! - The composed name is what a screen reader announces for such a cell
//!   anyway, the group's header and then the column's. The separator is the one
//!   the element joins a path with.
//!
//! A pivot without a column dimension has one header row in the element too,
//! and its CSV header is that row.
//!
//! # A subtotal row
//!
//! Its label goes into the first dimension column, where the element's row
//! header starts; the dimension columns it spans are **empty** fields. With
//! non-empty texts that reads one way only: a dimension cell of a data row is
//! then never empty — NULL and the empty string have labels of their own — so
//! an empty field in a dimension column means "spanned", whatever
//! [`CsvOptions::null`] says. A page that sets a label to `""` gives that up.
//!
//! **The CSV has no level column**, as the table has none: a subtotal is known
//! by its label alone. A group that is literally named `Total` looks like the
//! grand total — in the file as on the screen, where `data-level` tells them
//! apart for a script but not for a reader.
//!
//! # Whole, not in pieces
//!
//! A pivot is bounded (256 columns, 2 000 rows in V1) and the element holds all
//! of it, so this writes the whole thing as one string; there is no second
//! piece to write.

use opengrid_pivot::PivotResult;
use opengrid_types::{DataType, Value};

use crate::csv::{CsvOptions, cell, text_cell};
use crate::plain;

/// Between the values of a path and before the measure in a composed header.
///
/// The element joins a path with the same separator for its group header
/// (`crates/opengrid-web-components/src/pivot.rs`), so the CSV names a column
/// in the words the table uses.
pub const PATH_SEPARATOR: &str = " · ";

/// The words a pivot's headers use where the data has none.
///
/// The element's texts (`set_texts`), so that an export reads like the table
/// it came from: the browser implements this over the texts of the element
/// being exported, with the functions the element renders with.
pub trait PivotLabels {
    /// A dimension value as a header reads it; `None` is NULL. NULL and the
    /// empty string are two groups (S10, S14), and neither may be an empty
    /// header.
    fn dimension(&self, value: Option<&str>) -> String;
    /// The row header of the grand total.
    fn total(&self) -> String;
    /// The row header of the subtotal that closes `value` — which is already
    /// a header, as [`PivotLabels::dimension`] wrote it.
    fn subtotal(&self, value: &str) -> String;
}

/// The whole pivot as CSV: the byte order mark (when asked for), one header
/// line, then every row, each line ending in CRLF.
///
/// **Precondition**: `pivot` has the shape the engine gives it — the row
/// dimensions' columns first, then exactly one column per [`PivotColumn`],
/// and one level per row, none deeper than the row dimensions. It returns a
/// string, not a `Result`, because the shape is checked where a pivot comes in
/// from outside: [`pivot_from_json`] refuses any other with a sentence. A pivot
/// built by hand that breaks it panics here.
///
/// [`PivotColumn`]: opengrid_pivot::PivotColumn
/// [`pivot_from_json`]: opengrid_pivot::pivot_from_json
pub fn pivot_csv(pivot: &PivotResult, labels: &impl PivotLabels, options: &CsvOptions) -> String {
    let fields = pivot.data.schema.fields();
    // The row-dimension columns come first, one generated column per
    // `PivotColumn` after them (`PivotResult::data`, the precondition).
    let dimensions = fields.len() - pivot.columns.len();
    let delimiter = options.delimiter.to_string();

    let mut out = String::new();
    if options.bom {
        out.push('\u{FEFF}');
    }
    let mut header: Vec<String> = fields[..dimensions]
        .iter()
        .map(|field| field.name.as_str().to_owned())
        .collect();
    header.extend(pivot.columns.iter().map(|column| {
        let mut parts: Vec<String> = column
            .path
            .iter()
            .map(|value| labels.dimension(plain(value).as_deref()))
            .collect();
        parts.push(column.measure.as_str().to_owned());
        parts.join(PATH_SEPARATOR)
    }));
    // A header cell holds a label, and a column value in it is data: guarded.
    let header: Vec<String> = header
        .into_iter()
        .map(|name| text_cell(name, true, options))
        .collect();
    out.push_str(&header.join(&delimiter));
    out.push_str("\r\n");

    let text_columns: Vec<bool> = fields
        .iter()
        .map(|field| field.data_type == DataType::Utf8)
        .collect();
    for (row, level) in pivot.row_levels.iter().enumerate() {
        let level = usize::from(*level);
        let mut cells: Vec<String> = Vec::with_capacity(fields.len());
        if level < dimensions {
            // A subtotal, or the grand total at level 0: its label, then the
            // dimension columns it spans.
            let label = if level == 0 {
                labels.total()
            } else {
                let closed = &pivot.data.columns[level - 1][row];
                labels.subtotal(&labels.dimension(plain(closed).as_deref()))
            };
            cells.push(text_cell(label, true, options));
            cells.extend((1..dimensions).map(|_| String::new()));
        } else {
            for (column, text_column) in text_columns.iter().enumerate().take(dimensions) {
                cells.push(dimension_cell(
                    &pivot.data.columns[column][row],
                    *text_column,
                    labels,
                    options,
                ));
            }
        }
        for (column, text_column) in text_columns.iter().enumerate().skip(dimensions) {
            cells.push(cell(
                &pivot.data.columns[column][row],
                *text_column,
                options,
            ));
        }
        out.push_str(&cells.join(&delimiter));
        out.push_str("\r\n");
    }
    out
}

/// A dimension value of a data row, as its row header reads it.
///
/// The value's own text is guarded only in a text column — a year of `-5` is a
/// number — while a label that stands in for NULL or the empty string is text
/// whatever the column holds.
fn dimension_cell(
    value: &Value,
    text_column: bool,
    labels: &impl PivotLabels,
    options: &CsvOptions,
) -> String {
    let text = plain(value);
    let own = text.as_deref().is_some_and(|text| !text.is_empty());
    text_cell(
        labels.dimension(text.as_deref()),
        text_column || !own,
        options,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use opengrid_datasource::QueryResult;
    use opengrid_pivot::PivotColumn;
    use opengrid_types::{Field, FieldName, Schema};

    /// The element's English defaults.
    struct English;

    impl PivotLabels for English {
        fn dimension(&self, value: Option<&str>) -> String {
            match value {
                None => "(no value)".to_owned(),
                Some("") => "(empty)".to_owned(),
                Some(text) => text.to_owned(),
            }
        }
        fn total(&self) -> String {
            "Total".to_owned()
        }
        fn subtotal(&self, value: &str) -> String {
            format!("Total {value}")
        }
    }

    /// Labels a page could set, each starting like a formula.
    struct Hostile;

    impl PivotLabels for Hostile {
        fn dimension(&self, value: Option<&str>) -> String {
            match value {
                None => "=none".to_owned(),
                Some("") => "-empty".to_owned(),
                Some(text) => text.to_owned(),
            }
        }
        fn total(&self) -> String {
            "@total".to_owned()
        }
        fn subtotal(&self, value: &str) -> String {
            format!("{value} subtotal")
        }
    }

    fn field(name: &str, data_type: DataType) -> Field {
        Field::new(FieldName::new(name).unwrap(), data_type)
    }

    fn text(value: &str) -> Value {
        Value::Utf8(value.to_owned())
    }

    fn plain_options() -> CsvOptions {
        CsvOptions {
            bom: false,
            ..CsvOptions::default()
        }
    }

    /// Two row dimensions (a text and a number), one column dimension with a
    /// NULL value, one measure: a detail row, a subtotal, the grand total.
    fn pivot(customer: Value, week: Value) -> PivotResult {
        let fields = vec![
            field("customer", DataType::Utf8),
            field("week", DataType::Int64),
            field("n_0", DataType::Int64),
            field("n_1", DataType::Int64),
        ];
        let columns = vec![
            vec![customer.clone(), customer, Value::Null],
            vec![week, Value::Null, Value::Null],
            vec![Value::Int64(1), Value::Int64(1), Value::Int64(3)],
            vec![Value::Null, Value::Null, Value::Int64(2)],
        ];
        PivotResult {
            data: QueryResult::new(Schema::new(fields), columns, 3),
            row_levels: vec![2, 1, 0],
            columns: vec![
                PivotColumn {
                    path: vec![Value::Int64(2025)],
                    measure: FieldName::new("n").unwrap(),
                },
                PivotColumn {
                    path: vec![Value::Null],
                    measure: FieldName::new("n").unwrap(),
                },
            ],
        }
    }

    #[test]
    fn the_header_is_one_line_and_a_subtotal_spans_the_dimensions() {
        let csv = pivot_csv(
            &pivot(text("Alpha"), Value::Int64(7)),
            &English,
            &plain_options(),
        );
        assert_eq!(
            csv,
            "customer,week,2025 · n,(no value) · n\r\n\
             Alpha,7,1,\r\n\
             Total Alpha,,1,\r\n\
             Total,,3,2\r\n"
        );
    }

    #[test]
    fn a_spanned_dimension_stays_empty_when_null_is_spelled() {
        let options = CsvOptions {
            null: "\\N".to_owned(),
            ..plain_options()
        };
        let csv = pivot_csv(&pivot(text("Alpha"), Value::Int64(7)), &English, &options);
        // The measure's NULL is spelled; the cell the total's label spans is
        // not a value and stays empty.
        assert_eq!(csv.lines().nth(2), Some("Total Alpha,,1,\\N"), "{csv}");
        assert_eq!(csv.lines().nth(3), Some("Total,,3,2"), "{csv}");
    }

    #[test]
    fn null_and_the_empty_string_are_named_not_left_blank() {
        let csv = pivot_csv(&pivot(text(""), Value::Null), &English, &plain_options());
        assert_eq!(csv.lines().nth(1), Some("(empty),(no value),1,"), "{csv}");
        assert_eq!(csv.lines().nth(2), Some("Total (empty),,1,"), "{csv}");
    }

    #[test]
    fn a_label_is_guarded_and_a_number_dimension_is_not() {
        // A formula in the data, in a row header and in a column header.
        let mut hostile = pivot(text("=cmd"), Value::Int64(-5));
        hostile.columns[0].path = vec![text("+1")];
        let csv = pivot_csv(&hostile, &Hostile, &plain_options());
        assert_eq!(
            csv,
            "customer,week,'+1 · n,'=none · n\r\n\
             '=cmd,-5,1,\r\n\
             '=cmd subtotal,,1,\r\n\
             '@total,,3,2\r\n"
        );
        // NULL and the empty string in a **number** column: the label is text.
        let labelled = pivot_csv(&pivot(text(""), Value::Null), &Hostile, &plain_options());
        assert_eq!(
            labelled.lines().nth(1),
            Some("'-empty,'=none,1,"),
            "{labelled}"
        );

        let raw = CsvOptions {
            protect_formulas: false,
            ..plain_options()
        };
        let unguarded = pivot_csv(&hostile, &Hostile, &raw);
        assert_eq!(unguarded.lines().nth(1), Some("=cmd,-5,1,"), "{unguarded}");
    }

    #[test]
    fn a_subtotal_names_the_group_it_closes_not_the_outermost() {
        // Three row dimensions: the subtotal at level 2 closes `city`.
        let data = QueryResult::new(
            Schema::new(vec![
                field("country", DataType::Utf8),
                field("city", DataType::Utf8),
                field("shop", DataType::Utf8),
                field("n", DataType::Int64),
            ]),
            vec![
                vec![text("DE"), text("DE")],
                vec![text("Kiel"), text("Kiel")],
                vec![text("North"), Value::Null],
                vec![Value::Int64(2), Value::Int64(2)],
            ],
            2,
        );
        let pivot = PivotResult {
            data,
            row_levels: vec![3, 2],
            columns: vec![PivotColumn {
                path: Vec::new(),
                measure: FieldName::new("n").unwrap(),
            }],
        };
        assert_eq!(
            pivot_csv(&pivot, &English, &plain_options()),
            "country,city,shop,n\r\nDE,Kiel,North,2\r\nTotal Kiel,,,2\r\n"
        );
    }

    #[test]
    fn the_mark_and_the_delimiter_are_the_options() {
        let options = CsvOptions {
            delimiter: ';',
            ..CsvOptions::default()
        };
        let csv = pivot_csv(&pivot(text("a;b"), Value::Int64(7)), &English, &options);
        assert!(csv.starts_with("\u{FEFF}customer;week;2025 · n;"), "{csv}");
        assert_eq!(csv.lines().nth(2), Some("\"Total a;b\";;1;"), "{csv}");
    }

    #[test]
    fn a_pivot_without_a_column_dimension_is_named_by_its_measures() {
        let data = QueryResult::new(
            Schema::new(vec![
                field("country", DataType::Utf8),
                field("n", DataType::Int64),
            ]),
            vec![
                vec![text("DE"), Value::Null],
                vec![Value::Int64(2), Value::Int64(2)],
            ],
            2,
        );
        let pivot = PivotResult {
            data,
            row_levels: vec![1, 0],
            columns: vec![PivotColumn {
                path: Vec::new(),
                measure: FieldName::new("n").unwrap(),
            }],
        };
        assert_eq!(
            pivot_csv(&pivot, &English, &plain_options()),
            "country,n\r\nDE,2\r\nTotal,2\r\n"
        );
    }
}
