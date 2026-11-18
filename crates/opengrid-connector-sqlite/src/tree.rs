//! One level of a tree, answered by SQLite (issue #132, rules T1–T7).
//!
//! The engine's way — every row of the tree's scope, then the level in the
//! engine — moves the whole scope for each level a reader opens. Here SQLite
//! answers the level itself with `WITH RECURSIVE`. The statements are the
//! shared compiler's (`opengrid_sql::Compiler::tree_*`); this runs them and
//! turns their cells back into values. A flat tree (T8) answers with the whole
//! visible tree anyway, so it keeps the engine's way.
//!
//! Every statement reads the same `scoped` rows: the table under the tree's
//! scope, with three columns of its own — the key, the parent and whether the
//! row matches the filter — copied once into a temporary table indexed on the
//! key and the parent. Without the indexes each recursion step would read the
//! whole tree, and a tree of a hundred thousand nodes takes minutes. The
//! source's own file is not touched: temporary tables live in SQLite's
//! temporary database.

use opengrid_connector::{DataSourceError, QueryResult, Value};
use opengrid_datasource::TreeLevel;
use opengrid_query::{AggregateFn, ValidatedQuery, ValidatedTree};
use opengrid_sql::{CompiledQuery, Dialect};
use rusqlite::Connection;
use rusqlite::types::Value as Sql;

use crate::{SqlCompiler, compile, data, params, read, sqlite};

/// The temporary table the statements read (`Dialect::tree_table`).
const TEMP: &str = "og_tree_scoped";
const KEY: &str = "__og_key";

/// All the rows of a statement, each cell as SQLite answers it.
fn rows(
    connection: &Connection,
    statement: &CompiledQuery,
    width: usize,
) -> Result<Vec<Vec<Sql>>, DataSourceError> {
    let mut prepared = connection.prepare(&statement.sql).map_err(sqlite)?;
    let mut cursor = prepared
        .query(rusqlite::params_from_iter(params(statement)?.iter()))
        .map_err(sqlite)?;
    let mut out = Vec::new();
    while let Some(row) = cursor.next().map_err(sqlite)? {
        out.push(
            (0..width)
                .map(|index| row.get(index).map_err(sqlite))
                .collect::<Result<Vec<Sql>, _>>()?,
        );
    }
    Ok(out)
}

fn number(cell: Option<&Sql>) -> u64 {
    match cell {
        Some(Sql::Integer(value)) => u64::try_from(*value).unwrap_or(0),
        _ => 0,
    }
}

/// Answers one level of `tree` (T1–T7) in SQLite.
pub(crate) fn level(
    compiler: &SqlCompiler,
    connection: &Connection,
    query: &ValidatedQuery,
    tree: &ValidatedTree,
) -> Result<QueryResult, DataSourceError> {
    connection
        .execute_batch(&format!("DROP TABLE IF EXISTS temp.{TEMP}"))
        .map_err(sqlite)?;
    let prepare = compiler.tree_prepare(query, tree).map_err(compile)?;
    connection
        .execute(
            &prepare.sql,
            rusqlite::params_from_iter(params(&prepare)?.iter()),
        )
        .map_err(sqlite)?;
    let answer = connection
        .execute_batch(compiler.dialect().tree_indexes())
        .map_err(sqlite)
        .and_then(|()| answer(compiler, connection, query, tree));
    // Gone again whatever the answer: the next tree copies its own rows.
    let _ = connection.execute_batch(&format!("DROP TABLE IF EXISTS temp.{TEMP}"));
    answer
}

fn answer(
    compiler: &SqlCompiler,
    connection: &Connection,
    query: &ValidatedQuery,
    tree: &ValidatedTree,
) -> Result<QueryResult, DataSourceError> {
    let key_type = compiler
        .schema()
        .data_type(tree.key.as_str())
        .ok_or_else(|| crate::backend(format!("no column {:?}", tree.key.as_str())))?;
    let first_key = |statement: CompiledQuery| -> Result<Option<Value>, DataSourceError> {
        match rows(connection, &statement, 1)?.into_iter().next() {
            None => Ok(None),
            Some(mut row) => Ok(Some(read(row.remove(0), key_type, KEY)?)),
        }
    };

    // T1: every key once.
    if let Some(key) = first_key(compiler.tree_duplicate_key(tree).map_err(compile)?)? {
        return Err(data(opengrid_datasource::tree_duplicate_key(&key)));
    }
    // T3: every node is reached from a root.
    if let Some(key) = first_key(compiler.tree_cycle(tree).map_err(compile)?)? {
        return Err(data(opengrid_datasource::tree_cycle(&key)));
    }

    // T2 and T5: the orphans and the matches, over the whole tree; and
    // whether `under` exists (T4).
    let counts = rows(connection, &compiler.tree_counts(tree).map_err(compile)?, 3)?;
    let count = counts.first();
    let (orphans, matches) = (
        number(count.and_then(|row| row.first())),
        number(count.and_then(|row| row.get(1))),
    );
    let aggregate_schema = if tree.aggregate.is_empty() {
        Default::default()
    } else {
        tree.aggregate_schema.clone()
    };
    if number(count.and_then(|row| row.get(2))) == 0 {
        // A node that does not exist has no children.
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

    let total = rows(connection, &compiler.tree_total(tree).map_err(compile)?, 1)?
        .first()
        .and_then(|row| row.first())
        .map_or(0, |cell| number(Some(cell)));

    // The level: the shown children of `under`, or the shown roots — with each
    // node's shown children counted, sorted among siblings (T6, ties by the
    // key) and paged — and the subtree aggregates (T7).
    let (statement, output) = compiler.tree_level(query, tree).map_err(compile)?;
    let width = query.output_schema.len();
    let mut columns: Vec<Vec<Value>> = vec![Vec::new(); width];
    let mut children = Vec::new();
    let mut matched = Vec::new();
    let mut aggregates: Vec<Vec<Value>> = vec![Vec::new(); tree.aggregate.len()];
    for row in rows(connection, &statement, output.len())? {
        for (index, cell) in row.into_iter().enumerate() {
            let (name, data_type) = &output[index];
            let value = read(cell, *data_type, name)?;
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
                // A node with no match at or below it (S11): nothing to count,
                // nothing else to aggregate.
                let position = index - width - 2;
                aggregates[position].push(match (value, tree.aggregate[position].function) {
                    (Value::Null, AggregateFn::Count) => Value::Int64(0),
                    (value, _) => value,
                });
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
