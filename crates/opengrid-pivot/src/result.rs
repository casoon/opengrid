//! The shape of a pivot's answer.

use opengrid_datasource::QueryResult;
use opengrid_types::{FieldName, Value};

/// One generated leaf column: which column-dimension values it stands for, and
/// which measure it holds.
///
/// The **name** of the column in [`PivotResult::data`] is positional
/// (`total_0`, `total_1`, …) because a data value is not an identifier — `2026`
/// does not start with a letter, NULL has no spelling, and the empty string is
/// not a name. The meaning lives here, in `path`, where a UI can render it as
/// a heading and a screen reader can read it.
#[derive(Clone, Debug, PartialEq)]
pub struct PivotColumn {
    /// The column-dimension values, outermost first. Empty when the pivot has
    /// no column dimension.
    pub path: Vec<Value>,
    /// The measure alias, as the pivot query declared it.
    pub measure: FieldName,
}

/// A pivot's answer: a table, plus the two things a table cannot say.
///
/// The cells are an ordinary [`QueryResult`] — column-oriented, Arrow-free
/// (E14), the same shape the wire format carries (E17) — so everything that
/// already handles a result handles this one. Beside it stand the two pieces of
/// structure a flat schema cannot hold: how deep each row is, and what each
/// column stands for.
#[derive(Clone, Debug, PartialEq)]
pub struct PivotResult {
    /// Row-dimension columns first, then one column per [`PivotColumn`].
    pub data: QueryResult,
    /// Per row: how many row dimensions are set.
    ///
    /// `rows.len()` is a detail row, `0` is the grand total, anything between is
    /// a subtotal. **A subtotal is not marked by NULL**: rule S10 gives NULL its
    /// own group, so a NULL in a dimension column is a real value and this
    /// number is the only thing that tells the two apart. It is the same job
    /// `GROUPING()` does in SQL.
    pub row_levels: Vec<u16>,
    /// One per generated column, in the order they appear in `data`.
    pub columns: Vec<PivotColumn>,
}

impl PivotResult {
    /// How many rows the pivot has, subtotals and grand total included.
    pub fn row_count(&self) -> usize {
        self.row_levels.len()
    }

    /// Whether the row at `index` is a subtotal or the grand total.
    pub fn is_total(&self, index: usize, row_dimensions: usize) -> bool {
        self.row_levels
            .get(index)
            .is_some_and(|level| usize::from(*level) < row_dimensions)
    }
}

/// The JSON form of a pivot answer (plan point 53).
///
/// The cells travel as the ordinary result form of E17, under `result`, so a
/// reader that already knows that shape needs to learn only two new keys: the
/// level per row and what each generated column stands for. A pivot is an
/// arrangement of a result, and the wire form says so.
pub fn pivot_to_json(result: &PivotResult, row_dimensions: &[FieldName]) -> String {
    let columns: Vec<serde_json::Value> = result
        .columns
        .iter()
        .map(|column| {
            serde_json::json!({
                "path": column.path,
                "measure": column.measure.as_str(),
            })
        })
        .collect();

    let body = serde_json::json!({
        "row_dimensions": row_dimensions
            .iter()
            .map(FieldName::as_str)
            .collect::<Vec<_>>(),
        "columns": columns,
        "levels": result.row_levels,
        "result": serde_json::from_str::<serde_json::Value>(
            &opengrid_datasource::wire::result_to_json(&result.data),
        )
        .expect("the result form is JSON"),
    });
    body.to_string()
}

/// A pivot answer that could not be read back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PivotReadError(String);

impl PivotReadError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    /// What was wrong with the answer.
    pub fn message(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for PivotReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PivotReadError {}

/// Reads the JSON form of a pivot answer back — the other half of
/// [`pivot_to_json`] — with its row dimensions.
///
/// **Strict**, as [`result_from_json`](opengrid_datasource::wire::result_from_json)
/// is: the answer may come from a page's own provider, and whatever reads it
/// next indexes the cells by its shape. So the shape is checked here, once, and
/// is an error with a sentence rather than a panic later: the row-dimension
/// columns first and named as the dimensions, then exactly one column per
/// generated column, one level per row, no level deeper than the dimensions.
///
/// **A path comes back untyped.** The wire form writes a column's path as bare
/// JSON scalars, without the column dimension's type, so a number is `Int64` or
/// `Float64`, a string `Utf8` — a date comes back as its text. That text is the
/// one a typed value would print, which is all a heading needs; the cells are
/// typed, because the result form carries their types.
pub fn pivot_from_json(json: &str) -> Result<(PivotResult, Vec<FieldName>), PivotReadError> {
    use serde_json::Value as Json;

    let body: Json = serde_json::from_str(json)
        .map_err(|error| PivotReadError::new(format!("pivot JSON: {error}")))?;
    let row_dimensions = body["row_dimensions"]
        .as_array()
        .ok_or_else(|| PivotReadError::new("pivot has no row_dimensions"))?
        .iter()
        .map(|name| {
            name.as_str()
                .and_then(|name| FieldName::new(name).ok())
                .ok_or_else(|| PivotReadError::new(format!("row dimension {name}: not a name")))
        })
        .collect::<Result<Vec<FieldName>, _>>()?;
    let row_levels = body["levels"]
        .as_array()
        .ok_or_else(|| PivotReadError::new("pivot has no levels"))?
        .iter()
        .map(|level| {
            level
                .as_u64()
                .and_then(|level| u16::try_from(level).ok())
                .filter(|level| usize::from(*level) <= row_dimensions.len())
                .ok_or_else(|| {
                    PivotReadError::new(format!(
                        "level {level}: a number from 0 to {}",
                        row_dimensions.len()
                    ))
                })
        })
        .collect::<Result<Vec<u16>, _>>()?;
    let columns = body["columns"]
        .as_array()
        .ok_or_else(|| PivotReadError::new("pivot has no columns"))?
        .iter()
        .map(|column| {
            let measure = column["measure"]
                .as_str()
                .and_then(|name| FieldName::new(name).ok())
                .ok_or_else(|| PivotReadError::new("a column has no measure"))?;
            let path = column["path"]
                .as_array()
                .ok_or_else(|| PivotReadError::new(format!("column {measure} has no path")))?
                .iter()
                .map(|value| match value {
                    Json::Null => Ok(Value::Null),
                    Json::Bool(flag) => Ok(Value::Bool(*flag)),
                    Json::Number(number) => number
                        .as_i64()
                        .map(Value::Int64)
                        .or_else(|| number.as_f64().map(Value::Float64))
                        .ok_or_else(|| PivotReadError::new(format!("path value {number}"))),
                    Json::String(text) => Ok(Value::Utf8(text.clone())),
                    other => Err(PivotReadError::new(format!(
                        "column {measure}: path value {other} is not a scalar"
                    ))),
                })
                .collect::<Result<Vec<Value>, _>>()?;
            Ok(PivotColumn { path, measure })
        })
        .collect::<Result<Vec<PivotColumn>, PivotReadError>>()?;
    let data = opengrid_datasource::wire::result_from_json(&body["result"].to_string())
        .map_err(|error| PivotReadError::new(format!("pivot cells: {error}")))?;

    let fields = data.schema.fields();
    let needed = row_dimensions.len() + columns.len();
    if fields.len() != needed {
        return Err(PivotReadError::new(format!(
            "pivot cells have {} columns, {} row dimensions and {} generated columns need {needed}",
            fields.len(),
            row_dimensions.len(),
            columns.len(),
        )));
    }
    if let Some((field, dimension)) = fields
        .iter()
        .zip(&row_dimensions)
        .find(|(field, dimension)| field.name != **dimension)
    {
        return Err(PivotReadError::new(format!(
            "pivot cell column {} stands where row dimension {dimension} belongs",
            field.name
        )));
    }
    if row_levels.len() != data.row_count() {
        return Err(PivotReadError::new(format!(
            "pivot has {} levels for {} rows",
            row_levels.len(),
            data.row_count()
        )));
    }

    Ok((
        PivotResult {
            data,
            row_levels,
            columns,
        },
        row_dimensions,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use opengrid_types::{DataType, Field, Schema};

    fn name(text: &str) -> FieldName {
        FieldName::new(text).unwrap()
    }

    fn text(value: &str) -> Value {
        Value::Utf8(value.to_owned())
    }

    /// Two row dimensions, a column dimension with a NULL value, a subtotal.
    fn pivot() -> PivotResult {
        let fields = vec![
            Field::new(name("country"), DataType::Utf8),
            Field::new(name("customer"), DataType::Utf8),
            Field::new(name("n_0"), DataType::Int64),
            Field::new(name("n_1"), DataType::Int64),
        ];
        let columns = vec![
            vec![text("DE"), text("DE"), Value::Null],
            vec![text(""), Value::Null, Value::Null],
            vec![Value::Int64(1), Value::Int64(1), Value::Int64(3)],
            vec![Value::Null, Value::Null, Value::Int64(2)],
        ];
        PivotResult {
            data: QueryResult::new(Schema::new(fields), columns, 3),
            row_levels: vec![2, 1, 0],
            columns: vec![
                PivotColumn {
                    path: vec![Value::Int64(2025)],
                    measure: name("n"),
                },
                PivotColumn {
                    path: vec![Value::Null],
                    measure: name("n"),
                },
            ],
        }
    }

    fn dimensions() -> Vec<FieldName> {
        vec![name("country"), name("customer")]
    }

    #[test]
    fn the_json_form_reads_back_to_the_pivot_it_was_written_from() {
        let json = pivot_to_json(&pivot(), &dimensions());
        let (read, rows) = pivot_from_json(&json).expect("reads back");
        assert_eq!(rows, dimensions());
        assert_eq!(read.row_levels, pivot().row_levels);
        assert_eq!(read.columns, pivot().columns);
        assert_eq!(read.data.columns, pivot().data.columns);
        assert_eq!(pivot_to_json(&read, &rows), json);
    }

    #[test]
    fn a_path_comes_back_untyped_with_the_same_text() {
        let mut dated = pivot();
        dated.columns[0].path = vec![Value::from_wire_str("2025-01-31", &DataType::Date).unwrap()];
        dated.columns[1].path = vec![Value::Float64(1.5)];
        let (read, _) = pivot_from_json(&pivot_to_json(&dated, &dimensions())).unwrap();
        assert_eq!(read.columns[0].path, vec![text("2025-01-31")]);
        assert_eq!(read.columns[1].path, vec![Value::Float64(1.5)]);
    }

    /// What a page's own provider might send: each is an error with a
    /// sentence, never a shape that panics whoever reads it next.
    #[test]
    fn a_pivot_of_the_wrong_shape_is_an_error() {
        let good: serde_json::Value =
            serde_json::from_str(&pivot_to_json(&pivot(), &dimensions())).unwrap();
        let broken = |change: &dyn Fn(&mut serde_json::Value)| {
            let mut body = good.clone();
            change(&mut body);
            pivot_from_json(&body.to_string())
                .expect_err("a broken pivot is refused")
                .message()
                .to_owned()
        };

        let more_columns = broken(&|body| {
            let extra = body["columns"][0].clone();
            body["columns"].as_array_mut().unwrap().push(extra);
        });
        assert!(more_columns.contains("need 5"), "{more_columns}");
        let more_dimensions = broken(&|body| {
            body["row_dimensions"]
                .as_array_mut()
                .unwrap()
                .push("x".into());
        });
        assert!(more_dimensions.contains("need 5"), "{more_dimensions}");
        let renamed = broken(&|body| body["row_dimensions"][1] = "city".into());
        assert!(renamed.contains("city"), "{renamed}");
        let short = broken(&|body| {
            body["levels"].as_array_mut().unwrap().pop();
        });
        assert!(short.contains("2 levels for 3 rows"), "{short}");
        let deep = broken(&|body| body["levels"][0] = 3.into());
        assert!(deep.contains("from 0 to 2"), "{deep}");
        let nested = broken(&|body| body["columns"][0]["path"][0] = serde_json::json!([1]));
        assert!(nested.contains("not a scalar"), "{nested}");
        let untyped = broken(&|body| body["result"]["columns"][2]["values"][0] = "x".into());
        assert!(untyped.contains("pivot cells"), "{untyped}");
    }
}
