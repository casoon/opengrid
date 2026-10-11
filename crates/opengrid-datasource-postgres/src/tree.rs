//! One level of a tree, answered by PostgreSQL (issue #132, rules T1–T7).
//!
//! The statements are the compiler's (`PostgresCompiler::tree_*`); this runs
//! them in one `REPEATABLE READ` transaction, so the checks, the counts and
//! the level all read the same rows — a temporary copy of the tree's own
//! columns, indexed, dropped with the transaction — and turns their text back
//! into values.

use opengrid_datasource::{DataSourceError, QueryResult, TreeLevel};
use opengrid_query::{ValidatedQuery, ValidatedTree};
use opengrid_types::{DataType, Value};
use tokio_postgres::IsolationLevel;
use tokio_postgres::types::ToSql;

use crate::PostgresDataSource;
use crate::compiler::CompiledQuery;
use crate::source::{backend, compile, text_params, texts, value_from_text};

/// A key as a sentence names it — the engine's words, so an error reads the
/// same whichever way the level was answered.
fn named(value: &Value) -> String {
    match value {
        Value::Utf8(text) => format!("{text:?}"),
        Value::Int64(number) => number.to_string(),
        Value::Float64(number) if number.is_nan() => "\"NaN\"".to_owned(),
        Value::Float64(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Decimal(decimal) => format!("{:?}", decimal.to_string()),
        Value::Date(date) => format!("{:?}", date.to_string()),
        Value::Timestamp(timestamp) => format!("{:?}", timestamp.to_string()),
        Value::Null => "null".to_owned(),
    }
}

impl PostgresDataSource {
    /// Answers one level of a tree in PostgreSQL.
    pub(crate) async fn execute_tree(
        &self,
        query: &ValidatedQuery,
        tree: &ValidatedTree,
    ) -> Result<QueryResult, DataSourceError> {
        let compiler = &self.compiler;
        let key_type = self
            .schema_of(tree.key.as_str())
            .ok_or_else(|| backend(format!("no column {:?}", tree.key.as_str())))?;

        let mut client = self.pool.get().await.map_err(backend)?;
        let transaction = client
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .start()
            .await
            .map_err(backend)?;
        let run = |compiled: CompiledQuery, width: usize| {
            let transaction = &transaction;
            async move {
                let params = text_params(&compiled.params);
                let refs: Vec<&(dyn ToSql + Sync)> = params
                    .iter()
                    .map(|param| param as &(dyn ToSql + Sync))
                    .collect();
                let rows = transaction
                    .query(&compiled.sql, &refs)
                    .await
                    .map_err(backend)?;
                Ok::<_, DataSourceError>(texts(&rows, width))
            }
        };

        // The tree's rows, once, indexed (see `PostgresCompiler::tree_prepare`);
        // gone again with the transaction.
        let prepare = compiler.tree_prepare(query, tree).map_err(compile)?;
        let params = text_params(&prepare.params);
        let refs: Vec<&(dyn ToSql + Sync)> = params
            .iter()
            .map(|param| param as &(dyn ToSql + Sync))
            .collect();
        transaction
            .execute(&prepare.sql, &refs)
            .await
            .map_err(backend)?;
        transaction
            .batch_execute(crate::compiler::TREE_INDEXES)
            .await
            .map_err(backend)?;

        // T1: every key once.
        let rows = run(compiler.tree_duplicate_key(tree).map_err(compile)?, 1).await?;
        if let Some(text) = rows.first().and_then(|row| row[0].clone()) {
            let key = value_from_text(Some(&text), key_type, tree.key.as_str())?;
            return Err(backend(format!(
                "tree: the key {} appears more than once (rule T1)",
                named(&key)
            )));
        }

        // T3: every node reached from a root.
        let rows = run(compiler.tree_cycle(tree).map_err(compile)?, 1).await?;
        if let Some(text) = rows.first().and_then(|row| row[0].clone()) {
            let key = value_from_text(Some(&text), key_type, tree.key.as_str())?;
            return Err(backend(format!(
                "tree: the parents of {} lead back to it — a cycle (rule T3)",
                named(&key)
            )));
        }

        // T2, T5 and whether `under` exists (T4).
        let rows = run(compiler.tree_counts(tree).map_err(compile)?, 3).await?;
        let number = |index: usize| -> u64 {
            rows.first()
                .and_then(|row| row[index].as_deref())
                .and_then(|text| text.parse().ok())
                .unwrap_or(0)
        };
        let (orphans, matches, under_exists) = (number(0), number(1), number(2) > 0);

        let aggregate_schema = if tree.aggregate.is_empty() {
            Default::default()
        } else {
            tree.aggregate_schema.clone()
        };
        if !under_exists {
            transaction.commit().await.map_err(backend)?;
            let mut result = QueryResult::new(
                query.output_schema.clone(),
                vec![Vec::new(); query.output_schema.len()],
                0,
            );
            result.tree = Some(TreeLevel {
                matches,
                orphans,
                aggregate_schema,
                aggregates: vec![Vec::new(); tree.aggregate.len()],
                ..TreeLevel::default()
            });
            return Ok(result);
        }

        let rows = run(compiler.tree_total(tree).map_err(compile)?, 1).await?;
        let total: u64 = rows
            .first()
            .and_then(|row| row[0].as_deref())
            .and_then(|text| text.parse().ok())
            .unwrap_or(0);

        let (statement, output) = compiler.tree_level(query, tree).map_err(compile)?;
        let rows = run(statement, output.len()).await?;
        transaction.commit().await.map_err(backend)?;

        // The query's columns, then children and match, then the aggregates.
        let width = query.output_schema.len();
        let mut columns = vec![Vec::with_capacity(rows.len()); width];
        let mut children = Vec::with_capacity(rows.len());
        let mut matched = Vec::with_capacity(rows.len());
        let mut aggregates = vec![Vec::with_capacity(rows.len()); tree.aggregate.len()];
        for row in rows {
            for (index, cell) in row.into_iter().enumerate() {
                let (name, data_type) = &output[index];
                let value = value_from_text(cell.as_deref(), *data_type, name)?;
                if index < width {
                    columns[index].push(value);
                } else if index == width {
                    children.push(match value {
                        Value::Int64(n) => u64::try_from(n).unwrap_or(0),
                        _ => 0,
                    });
                } else if index == width + 1 {
                    matched.push(value == Value::Bool(true));
                } else {
                    // A node with no match at or below it (S11): no rows to
                    // count, nothing else to aggregate.
                    let position = index - width - 2;
                    let value = match (value, tree.aggregate[position].function) {
                        (Value::Null, opengrid_query::AggregateFn::Count) => Value::Int64(0),
                        (value, _) => value,
                    };
                    aggregates[position].push(value);
                }
            }
        }

        let mut result = QueryResult::new(query.output_schema.clone(), columns, total);
        result.tree = Some(TreeLevel {
            children,
            matched,
            matches,
            orphans,
            aggregate_schema,
            aggregates,
            flat: None,
        });
        Ok(result)
    }

    /// The type of a column of the source's schema.
    fn schema_of(&self, name: &str) -> Option<DataType> {
        self.schema.data_type(name)
    }
}
