//! A table: a schema and one column per field, all of the same length.

use std::sync::Arc;

use opengrid_types::{Schema, Value};

use crate::Column;

/// Columns in schema order, with the invariant that every column has the type
/// its field declares and all have the same number of rows.
///
/// Columns are shared, not copied, between tables that hold them unchanged —
/// a projection ([`Table::select`]) or a clone costs no column data.
#[derive(Clone, Debug, PartialEq)]
pub struct Table {
    schema: Schema,
    columns: Vec<Arc<Column>>,
    rows: usize,
}

impl Table {
    /// Binds columns to a schema, checking count, types and lengths.
    ///
    /// The schema is stored with its derivations resolved away
    /// ([`Schema::materialized`]): once a derived column holds values, it is a
    /// column like any other.
    pub fn new(schema: &Schema, columns: Vec<Column>) -> Result<Self, String> {
        Self::from_shared(schema, columns.into_iter().map(Arc::new).collect())
    }

    fn from_shared(schema: &Schema, columns: Vec<Arc<Column>>) -> Result<Self, String> {
        if columns.len() != schema.len() {
            return Err(format!(
                "{} columns for {} fields",
                columns.len(),
                schema.len()
            ));
        }
        let rows = columns.first().map_or(0, |column| column.len());
        for (field, column) in schema.fields().iter().zip(&columns) {
            if column.data_type() != field.data_type {
                return Err(format!(
                    "column {} is {}, but the schema says {}",
                    field.name,
                    column.data_type(),
                    field.data_type
                ));
            }
            if column.len() != rows {
                return Err(format!(
                    "column {} has {} rows, the first has {rows}",
                    field.name,
                    column.len()
                ));
            }
        }
        Ok(Self {
            schema: schema.materialized(),
            columns,
            rows,
        })
    }

    /// The table's schema.
    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    /// The column at `index`, in schema order.
    pub fn column_at(&self, index: usize) -> &Column {
        &self.columns[index]
    }

    /// The column of the field `name`, if there is one.
    pub fn column(&self, name: &str) -> Option<&Column> {
        self.schema
            .index_of(name)
            .map(|index| self.column_at(index))
    }

    /// The columns `schema` names, in its order, sharing their data with this
    /// table. Every field has to exist here with the same type.
    pub fn select(&self, schema: &Schema) -> Result<Table, String> {
        let columns = schema
            .fields()
            .iter()
            .map(|field| {
                self.schema
                    .index_of(field.name.as_str())
                    .map(|index| Arc::clone(&self.columns[index]))
                    .ok_or_else(|| format!("no column named {:?}", field.name.as_str()))
            })
            .collect::<Result<Vec<_>, String>>()?;
        Self::from_shared(schema, columns)
    }

    /// Number of rows.
    pub fn num_rows(&self) -> usize {
        self.rows
    }

    /// The rows at `positions`, in that order.
    pub fn take(&self, positions: &[u32]) -> Table {
        Table {
            schema: self.schema.clone(),
            columns: self
                .columns
                .iter()
                .map(|column| Arc::new(column.take(positions)))
                .collect(),
            rows: positions.len(),
        }
    }

    /// `len` rows from `start` on; both are clamped to the table.
    pub fn slice(&self, start: usize, len: usize) -> Table {
        let start = start.min(self.rows);
        let len = len.min(self.rows - start);
        Table {
            schema: self.schema.clone(),
            columns: self
                .columns
                .iter()
                .map(|column| Arc::new(column.slice(start, len)))
                .collect(),
            rows: len,
        }
    }

    /// The typed values, one `Vec` per column — the column-oriented form of
    /// the storage-free `QueryResult` (E14).
    pub fn to_values(&self) -> Vec<Vec<Value>> {
        self.columns
            .iter()
            .map(|column| (0..self.rows).map(|row| column.value(row)).collect())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use opengrid_types::{DataType, Field, FieldName};

    use super::*;
    use crate::ColumnBuilder;

    fn int_column(values: &[i64]) -> Column {
        let mut builder = ColumnBuilder::new(DataType::Int64, values.len());
        for value in values {
            builder.push(&Value::Int64(*value)).unwrap();
        }
        builder.finish()
    }

    fn schema(names: &[&str]) -> Schema {
        Schema::new(
            names
                .iter()
                .map(|name| Field::new(FieldName::new(*name).unwrap(), DataType::Int64))
                .collect(),
        )
    }

    #[test]
    fn columns_of_different_length_are_refused() {
        let error = Table::new(
            &schema(&["a", "b"]),
            vec![int_column(&[1, 2]), int_column(&[1])],
        )
        .unwrap_err();
        assert_eq!(error, "column b has 1 rows, the first has 2");
    }

    #[test]
    fn a_column_of_the_wrong_type_is_refused() {
        let schema = Schema::new(vec![Field::new(
            FieldName::new("a").unwrap(),
            DataType::Utf8,
        )]);
        assert!(Table::new(&schema, vec![int_column(&[1])]).is_err());
    }

    #[test]
    fn select_shares_columns_in_the_order_asked() {
        let table = Table::new(
            &schema(&["a", "b"]),
            vec![int_column(&[1, 2]), int_column(&[3, 4])],
        )
        .unwrap();
        let picked = table.select(&schema(&["b"])).unwrap();
        assert_eq!(
            picked.to_values(),
            vec![vec![Value::Int64(3), Value::Int64(4)]]
        );
        assert!(Arc::ptr_eq(&picked.columns[0], &table.columns[1]));
        assert!(table.select(&schema(&["c"])).is_err());
    }

    #[test]
    fn slice_is_clamped_to_the_table() {
        let table = Table::new(&schema(&["a"]), vec![int_column(&[1, 2, 3])]).unwrap();
        assert_eq!(table.slice(2, 10).num_rows(), 1);
        assert_eq!(table.slice(5, 1).num_rows(), 0);
    }
}
