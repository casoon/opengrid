//! Loading and checking of conformance cases (`cases/*.json`).
//!
//! The dataset schema is a small JSON format owned by this crate — the engines
//! get their schema from the datasource configuration or from ingest (points 06,
//! 24), so nothing outside the suite needs to read it:
//!
//! ```json
//! { "fields": [ { "name": "id", "type": "int64", "nullable": false } ] }
//! ```
//!
//! Types are `bool`, `int64`, `float64`, `utf8`, `date`, `timestamp` and
//! `{ "decimal": { "precision": 12, "scale": 2 } }`. Building the schema goes
//! through [`DataType::decimal`], so an impossible precision is rejected here
//! just as it would be in code.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use opengrid_query::{Limits, Query, QueryError, ValidatedQuery};
use opengrid_types::{DataType, Field, FieldName, Schema, Value, ValueError};

use crate::Table;

/// One semantics case, as written in `cases/*.json`.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    /// Stable identifier, e.g. `s3-nulls-last-desc`.
    pub id: String,
    /// The semantics rule this case pins down, `S1` … `S14`.
    pub rule: String,
    pub query: Query,
    pub expected: ExpectedTable,
    /// Whether row order is part of the expectation.
    #[serde(default)]
    pub ordered: bool,
}

/// The expected result of a case, before its cells are typed.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedTable {
    /// Output column names, in order — must equal the query's output columns.
    pub columns: Vec<String>,
    /// One array per row. Cells are JSON scalars read against the column type.
    pub rows: Vec<Vec<serde_json::Value>>,
}

/// A case whose query validated and whose expectation is typed.
#[derive(Clone, Debug)]
pub struct Checked {
    pub case: Case,
    pub query: ValidatedQuery,
    pub expected: Table,
}

/// Anything that can go wrong while loading the suite.
#[derive(Debug)]
pub enum CaseError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
    Schema {
        path: PathBuf,
        message: String,
    },
    Query {
        id: String,
        source: QueryError,
    },
    Columns {
        id: String,
        expected: Vec<String>,
        actual: Vec<String>,
    },
    Cell {
        id: String,
        row: usize,
        column: usize,
        message: String,
    },
    DuplicateId {
        id: String,
    },
}

impl std::fmt::Display for CaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CaseError::Io { path, source } => write!(f, "{}: {source}", path.display()),
            CaseError::Json { path, source } => write!(f, "{}: {source}", path.display()),
            CaseError::Schema { path, message } => write!(f, "{}: {message}", path.display()),
            CaseError::Query { id, source } => write!(f, "case {id}: {source}"),
            CaseError::Columns {
                id,
                expected,
                actual,
            } => write!(
                f,
                "case {id}: expected columns {expected:?}, but the query produces {actual:?}"
            ),
            CaseError::Cell {
                id,
                row,
                column,
                message,
            } => write!(f, "case {id}: row {row}, column {column}: {message}"),
            CaseError::DuplicateId { id } => write!(f, "duplicate case id {id}"),
        }
    }
}

impl std::error::Error for CaseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CaseError::Io { source, .. } => Some(source),
            CaseError::Json { source, .. } => Some(source),
            CaseError::Query { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Loads the dataset schema.
pub fn load_schema(path: &Path) -> Result<Schema, CaseError> {
    let file: SchemaFile =
        serde_json::from_str(&read(path)?).map_err(|source| CaseError::Json {
            path: path.to_path_buf(),
            source,
        })?;
    file.into_schema().map_err(|message| CaseError::Schema {
        path: path.to_path_buf(),
        message,
    })
}

/// Validates one case and reads its expectation against the query's output types.
pub fn check_case(case: Case, schema: &Schema) -> Result<Checked, CaseError> {
    let query = case
        .query
        .validate(schema, &Limits::default())
        .map_err(|source| CaseError::Query {
            id: case.id.clone(),
            source,
        })?;

    let output = query.output_schema.fields();
    let actual: Vec<String> = output.iter().map(|field| field.name.to_string()).collect();
    if actual != case.expected.columns {
        return Err(CaseError::Columns {
            id: case.id.clone(),
            expected: case.expected.columns.clone(),
            actual,
        });
    }

    let mut rows = Vec::with_capacity(case.expected.rows.len());
    for (row_index, row) in case.expected.rows.iter().enumerate() {
        if row.len() != output.len() {
            return Err(CaseError::Cell {
                id: case.id.clone(),
                row: row_index,
                column: 0,
                message: format!("expected {} cells, found {}", output.len(), row.len()),
            });
        }
        let mut cells = Vec::with_capacity(row.len());
        for (column, cell) in row.iter().enumerate() {
            let value = Value::deserialize_typed(cell.clone(), &output[column].data_type).map_err(
                |error| CaseError::Cell {
                    id: case.id.clone(),
                    row: row_index,
                    column,
                    message: error.to_string(),
                },
            )?;
            cells.push(value);
        }
        rows.push(cells);
    }

    let columns = output.iter().map(|field| field.name.clone()).collect();
    Ok(Checked {
        case,
        query,
        expected: Table::new(columns, rows),
    })
}

/// Loads and checks every `*.json` case in a directory, in file-name order.
pub fn check_dir(dir: &Path, schema: &Schema) -> Result<Vec<Checked>, CaseError> {
    let entries = std::fs::read_dir(dir).map_err(|source| CaseError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    let mut paths = Vec::new();
    for entry in entries {
        let path = entry
            .map_err(|source| CaseError::Io {
                path: dir.to_path_buf(),
                source,
            })?
            .path();
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            paths.push(path);
        }
    }
    paths.sort();

    let mut seen = BTreeSet::new();
    let mut checked = Vec::with_capacity(paths.len());
    for path in paths {
        let case: Case = serde_json::from_str(&read(&path)?).map_err(|source| CaseError::Json {
            path: path.clone(),
            source,
        })?;
        if !seen.insert(case.id.clone()) {
            return Err(CaseError::DuplicateId {
                id: case.id.clone(),
            });
        }
        checked.push(check_case(case, schema)?);
    }
    Ok(checked)
}

/// The set of semantics rules covered by a set of checked cases.
pub fn rules_covered(checked: &[Checked]) -> BTreeSet<String> {
    checked
        .iter()
        .map(|checked| checked.case.rule.clone())
        .collect()
}

fn read(path: &Path) -> Result<String, CaseError> {
    std::fs::read_to_string(path).map_err(|source| CaseError::Io {
        path: path.to_path_buf(),
        source,
    })
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SchemaFile {
    fields: Vec<FieldRepr>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct FieldRepr {
    name: String,
    #[serde(rename = "type")]
    data_type: TypeRepr,
    #[serde(default)]
    nullable: bool,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum TypeRepr {
    Bool,
    Int64,
    Float64,
    Decimal { precision: u8, scale: u8 },
    Utf8,
    Date,
    Timestamp,
}

impl SchemaFile {
    fn into_schema(self) -> Result<Schema, String> {
        let mut fields = Vec::with_capacity(self.fields.len());
        for repr in self.fields {
            let name = FieldName::new(&repr.name)
                .map_err(|error| format!("field {:?}: {error}", repr.name))?;
            let data_type = repr
                .data_type
                .to_data_type()
                .map_err(|error| format!("field {:?}: {error}", repr.name))?;
            fields.push(if repr.nullable {
                Field::new(name, data_type)
            } else {
                Field::required(name, data_type)
            });
        }
        Ok(Schema::new(fields))
    }
}

impl TypeRepr {
    fn to_data_type(&self) -> Result<DataType, ValueError> {
        Ok(match self {
            TypeRepr::Bool => DataType::Bool,
            TypeRepr::Int64 => DataType::Int64,
            TypeRepr::Float64 => DataType::Float64,
            TypeRepr::Decimal { precision, scale } => DataType::decimal(*precision, *scale)?,
            TypeRepr::Utf8 => DataType::Utf8,
            TypeRepr::Date => DataType::Date,
            TypeRepr::Timestamp => DataType::Timestamp,
        })
    }
}
