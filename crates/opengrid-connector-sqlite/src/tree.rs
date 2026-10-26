//! One level of a tree, answered by SQLite (issue #132, rules T1–T7).
//!
//! The engine's way — every row of the tree's scope, then the level in the
//! engine — moves the whole scope for each level a reader opens. Here SQLite
//! answers the level itself with `WITH RECURSIVE`: the nodes shown (matches
//! and their ancestors, T5), their child counts (T4), the orphans (T2), the
//! checks for a key twice (T1) and for a cycle (T3), one level sorted and
//! paged (T6), and each node's subtree aggregates (T7). A flat tree (T8)
//! answers with the whole visible tree anyway, so it keeps the engine's way.
//!
//! Every statement reads the same `scoped` rows: the table under the tree's
//! scope, with three columns of its own — the key, the parent and whether the
//! row matches the filter — and its `rowid` as the row's identity, which a
//! NULL key cannot be. `UNION` (not `UNION ALL`) in every recursion, so a
//! cycle ends the walk instead of the statement; T3 then names it.

use opengrid_connector::{DataSourceError, QueryResult, Value};
use opengrid_datasource::TreeLevel;
use opengrid_query::{ValidatedQuery, ValidatedTree};
use opengrid_types::DataType;
use rusqlite::Connection;
use rusqlite::types::Value as Sql;

use crate::{Builder, Compiler, backend, quote, read, sort_key, sqlite};

const KEY: &str = "__og_key";
const PARENT: &str = "__og_parent";
const MATCH: &str = "__og_match";
const ROW: &str = "__og_row";
/// The temporary table a tree's statements read (see `Compiler::prepare`).
const TEMP: &str = "og_tree_scoped";
/// The source's own table, joined by row id for the columns a level shows.
const BASE: &str = "og_base";

impl Compiler<'_> {
    /// Copies the tree's rows — its row id, key, parent and whether it
    /// matches, nothing else — into a temporary table indexed on the key and
    /// the parent. Every recursion joins a node to its parent or its children;
    /// without the indexes each step would read the whole tree, and a tree of
    /// a hundred thousand nodes takes minutes. The source's own file is not
    /// touched: temporary tables live in SQLite's temporary database.
    fn prepare(
        &self,
        connection: &Connection,
        query: &ValidatedQuery,
        tree: &ValidatedTree,
    ) -> Result<(), DataSourceError> {
        connection
            .execute_batch(&format!("DROP TABLE IF EXISTS temp.{TEMP}"))
            .map_err(sqlite)?;
        let mut sql = Builder::default();
        sql.push(&format!(
            "CREATE TEMP TABLE {TEMP} AS SELECT rowid AS {ROW}, "
        ));
        sql.push(&self.column(tree.key.as_str())?);
        sql.push(&format!(" AS {KEY}, "));
        sql.push(&self.column(tree.parent.as_str())?);
        sql.push(&format!(" AS {PARENT}, "));
        match &query.filter {
            Some(filter) => {
                sql.push("CASE WHEN ");
                self.filter(filter, &mut sql)?;
                sql.push(" THEN 1 ELSE 0 END");
            }
            None => sql.push("1"),
        }
        sql.push(&format!(" AS {MATCH} FROM "));
        sql.push(&quote(self.table)?);
        if let Some(scope) = &tree.scope {
            sql.push(" WHERE ");
            self.filter(scope, &mut sql)?;
        }
        connection
            .execute(&sql.sql, rusqlite::params_from_iter(sql.params.iter()))
            .map_err(sqlite)?;
        connection
            .execute_batch(&format!(
                "CREATE INDEX temp.{TEMP}_key ON {TEMP}({KEY}); \
                 CREATE INDEX temp.{TEMP}_parent ON {TEMP}({PARENT}); \
                 CREATE UNIQUE INDEX temp.{TEMP}_row ON {TEMP}({ROW});"
            ))
            .map_err(sqlite)?;
        Ok(())
    }

    /// `WITH RECURSIVE scoped AS (…)`: the tree's rows as [`Self::prepare`]
    /// copied them.
    fn scoped(sql: &mut Builder) {
        sql.push(&format!(
            "WITH RECURSIVE scoped AS NOT MATERIALIZED (SELECT * FROM temp.{TEMP})"
        ));
    }

    /// `, shown(__og_r)`: the matches and every ancestor of one (T5).
    fn shown(sql: &mut Builder) {
        sql.push(&format!(
            ", shown(__og_r) AS (SELECT {ROW} FROM scoped WHERE {MATCH} = 1 \
             UNION SELECT p.{ROW} FROM shown JOIN scoped c ON c.{ROW} = shown.__og_r \
             JOIN scoped p ON p.{KEY} = c.{PARENT})"
        ));
    }
}

/// A root's condition on the row `c` (T2): no parent, or one that is no node.
fn is_root() -> String {
    format!(
        "(c.{PARENT} IS NULL OR NOT EXISTS (SELECT 1 FROM scoped p WHERE p.{KEY} = c.{PARENT}))"
    )
}

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
        // A list is never a tree key.
        Value::List(_) => opengrid_json::ToJson::to_json(value).to_string(),
    }
}

/// One count, from a statement that answers one integer.
fn count(connection: &Connection, sql: &Builder) -> Result<u64, DataSourceError> {
    let total: i64 = connection
        .query_row(
            &sql.sql,
            rusqlite::params_from_iter(sql.params.iter()),
            |row| row.get(0),
        )
        .map_err(sqlite)?;
    Ok(u64::try_from(total).unwrap_or(0))
}

/// The first key a statement answers, if any.
fn first_key(
    connection: &Connection,
    sql: &Builder,
    key_type: DataType,
) -> Result<Option<Value>, DataSourceError> {
    let mut prepared = connection.prepare(&sql.sql).map_err(sqlite)?;
    let mut rows = prepared
        .query(rusqlite::params_from_iter(sql.params.iter()))
        .map_err(sqlite)?;
    match rows.next().map_err(sqlite)? {
        None => Ok(None),
        Some(row) => {
            let cell: Sql = row.get(0).map_err(sqlite)?;
            Ok(Some(read(cell, key_type, KEY)?))
        }
    }
}

/// Answers one level of `tree` (T1–T7) in SQLite.
pub(crate) fn level(
    compiler: &Compiler<'_>,
    connection: &Connection,
    query: &ValidatedQuery,
    tree: &ValidatedTree,
) -> Result<QueryResult, DataSourceError> {
    compiler.prepare(connection, query, tree)?;
    let answer = answer(compiler, connection, query, tree);
    // Gone again whatever the answer: the next tree copies its own rows.
    let _ = connection.execute_batch(&format!("DROP TABLE IF EXISTS temp.{TEMP}"));
    answer
}

fn answer(
    compiler: &Compiler<'_>,
    connection: &Connection,
    query: &ValidatedQuery,
    tree: &ValidatedTree,
) -> Result<QueryResult, DataSourceError> {
    let key_type = compiler.data_type(tree.key.as_str())?;

    // T1: every key once.
    let mut sql = Builder::default();
    Compiler::scoped(&mut sql);
    sql.push(&format!(
        " SELECT {KEY} FROM scoped WHERE {KEY} IS NOT NULL GROUP BY {KEY} \
         HAVING count(*) > 1 ORDER BY {KEY} LIMIT 1"
    ));
    if let Some(key) = first_key(connection, &sql, key_type)? {
        return Err(backend(format!(
            "tree: the key {} appears more than once (rule T1)",
            named(&key)
        )));
    }

    // T3: every node is reached from a root; one that is not lies on a cycle
    // or below one, and the walk up from it names a node of the cycle.
    let mut sql = Builder::default();
    Compiler::scoped(&mut sql);
    sql.push(&format!(
        ", reached(__og_r) AS (SELECT c.{ROW} FROM scoped c WHERE {root} \
         UNION SELECT c.{ROW} FROM reached JOIN scoped p ON p.{ROW} = reached.__og_r \
         JOIN scoped c ON c.{PARENT} = p.{KEY}), \
         lost(__og_r) AS (SELECT s.{ROW} FROM scoped s WHERE NOT EXISTS \
             (SELECT 1 FROM reached WHERE reached.__og_r = s.{ROW})), \
         up(__og_s, __og_r, __og_d) AS (SELECT __og_r, __og_r, 0 FROM lost \
         UNION SELECT up.__og_s, p.{ROW}, up.__og_d + 1 FROM up JOIN scoped c ON c.{ROW} = up.__og_r \
         JOIN scoped p ON p.{KEY} = c.{PARENT} WHERE up.__og_d < (SELECT count(*) FROM lost)) \
         SELECT scoped.{KEY} FROM up JOIN scoped ON scoped.{ROW} = up.__og_s \
         WHERE up.__og_r = up.__og_s AND up.__og_d > 0 ORDER BY up.__og_s LIMIT 1",
        root = is_root()
    ));
    if let Some(key) = first_key(connection, &sql, key_type)? {
        return Err(backend(format!(
            "tree: the parents of {} lead back to it — a cycle (rule T3)",
            named(&key)
        )));
    }

    // T2 and T5: the orphans and the matches, over the whole tree.
    let mut sql = Builder::default();
    Compiler::scoped(&mut sql);
    sql.push(&format!(
        " SELECT count(*) FROM scoped c WHERE c.{PARENT} IS NOT NULL AND NOT EXISTS \
         (SELECT 1 FROM scoped p WHERE p.{KEY} = c.{PARENT})"
    ));
    let orphans = count(connection, &sql)?;
    let mut sql = Builder::default();
    Compiler::scoped(&mut sql);
    sql.push(&format!(" SELECT count(*) FROM scoped WHERE {MATCH} = 1"));
    let matches = count(connection, &sql)?;

    // T4: a node that does not exist has no children.
    if let Some(under) = &tree.under {
        let mut sql = Builder::default();
        Compiler::scoped(&mut sql);
        sql.push(&format!(" SELECT count(*) FROM scoped WHERE {KEY} = "));
        sql.param(under, key_type);
        if count(connection, &sql)? == 0 {
            return Ok(empty(query, tree, matches, orphans));
        }
    }

    // The level: the shown children of `under`, or the shown roots — with
    // each node's shown children counted, sorted among siblings (T6, ties by
    // the key) and paged.
    let members = |sql: &mut Builder| -> Result<(), DataSourceError> {
        Compiler::scoped(sql);
        Compiler::shown(sql);
        sql.push(&format!(
            ", members(__og_r) AS (SELECT c.{ROW} FROM scoped c JOIN shown ON shown.__og_r = c.{ROW} WHERE "
        ));
        match &tree.under {
            Some(under) => {
                sql.push(&format!("c.{PARENT} = "));
                sql.param(under, key_type);
            }
            None => sql.push(&is_root()),
        }
        sql.push(")");
        Ok(())
    };

    let mut sql = Builder::default();
    members(&mut sql)?;
    sql.push(" SELECT count(*) FROM members");
    let total = count(connection, &sql)?;

    let mut sql = Builder::default();
    members(&mut sql)?;
    sql.push(&format!(
        ", kids(__og_r, __og_n) AS (SELECT p.{ROW}, count(*) FROM shown JOIN scoped c ON c.{ROW} = shown.__og_r \
         JOIN scoped p ON p.{KEY} = c.{PARENT} GROUP BY p.{ROW}) SELECT "
    ));
    let fields = query.output_schema.fields();
    for field in fields {
        sql.push(&compiler.column(field.name.as_str())?);
        sql.push(" AS ");
        sql.push(&quote(field.name.as_str())?);
        sql.push(", ");
    }
    sql.push(&format!(
        "coalesce(kids.__og_n, 0), scoped.{MATCH}, scoped.{ROW} FROM members \
         JOIN scoped ON scoped.{ROW} = members.__og_r JOIN {table} AS {BASE} ON {BASE}.rowid = scoped.{ROW} LEFT JOIN kids ON kids.__og_r = scoped.{ROW} ORDER BY ",
        table = quote(compiler.table)?
    ));
    for key in &query.sort {
        sql.push(&sort_key(key)?);
        sql.push(", ");
    }
    sql.push(&format!("scoped.{KEY} ASC NULLS LAST"));
    if query.limit.is_some() || query.offset.is_some() {
        sql.push(" LIMIT ");
        sql.param(
            &Value::Int64(query.limit.map_or(-1, |limit| limit as i64)),
            DataType::Int64,
        );
        if let Some(offset) = query.offset {
            sql.push(" OFFSET ");
            sql.param(&Value::Int64(offset as i64), DataType::Int64);
        }
    }

    let mut columns: Vec<Vec<Value>> = vec![Vec::new(); fields.len()];
    let mut children = Vec::new();
    let mut matched = Vec::new();
    let mut rows = Vec::new();
    {
        let mut prepared = connection.prepare(&sql.sql).map_err(sqlite)?;
        let mut cursor = prepared
            .query(rusqlite::params_from_iter(sql.params.iter()))
            .map_err(sqlite)?;
        while let Some(row) = cursor.next().map_err(sqlite)? {
            for (index, field) in fields.iter().enumerate() {
                let cell: Sql = row.get(index).map_err(sqlite)?;
                columns[index].push(read(cell, field.data_type, field.name.as_str())?);
            }
            let n: i64 = row.get(fields.len()).map_err(sqlite)?;
            let hit: i64 = row.get(fields.len() + 1).map_err(sqlite)?;
            let id: i64 = row.get(fields.len() + 2).map_err(sqlite)?;
            children.push(u64::try_from(n).unwrap_or(0));
            matched.push(hit == 1);
            rows.push(id);
        }
    }

    let aggregates = subtree_aggregates(compiler, connection, tree, &rows)?;
    let mut result = QueryResult::new(query.output_schema.clone(), columns, total);
    result.tree = Some(TreeLevel {
        children,
        matched,
        matches,
        orphans,
        aggregate_schema: if tree.aggregate.is_empty() {
            Default::default()
        } else {
            tree.aggregate_schema.clone()
        },
        aggregates,
        flat: None,
    });
    Ok(result)
}

/// T7: per node of the page, its aggregates over the matches among itself
/// and its descendants — the same expressions as a grouped query (S12).
fn subtree_aggregates(
    compiler: &Compiler<'_>,
    connection: &Connection,
    tree: &ValidatedTree,
    rows: &[i64],
) -> Result<Vec<Vec<Value>>, DataSourceError> {
    if tree.aggregate.is_empty() {
        return Ok(Vec::new());
    }
    let fields = tree.aggregate_schema.fields();
    let mut by_row: std::collections::HashMap<i64, Vec<Value>> = std::collections::HashMap::new();
    if !rows.is_empty() {
        let mut sql = Builder::default();
        Compiler::scoped(&mut sql);
        sql.push(", top(__og_r) AS (VALUES ");
        for (index, row) in rows.iter().enumerate() {
            if index > 0 {
                sql.push(", ");
            }
            sql.push("(");
            sql.param(&Value::Int64(*row), DataType::Int64);
            sql.push(")");
        }
        sql.push(&format!(
            "), below(__og_t, __og_r) AS (SELECT __og_r, __og_r FROM top \
             UNION SELECT below.__og_t, c.{ROW} FROM below JOIN scoped p ON p.{ROW} = below.__og_r \
             JOIN scoped c ON c.{PARENT} = p.{KEY}) SELECT below.__og_t"
        ));
        for aggregate in &tree.aggregate {
            sql.push(", ");
            sql.push(&compiler.aggregate(aggregate)?);
        }
        sql.push(&format!(
            " FROM below JOIN scoped ON scoped.{ROW} = below.__og_r JOIN {table} AS {BASE} ON {BASE}.rowid = scoped.{ROW} WHERE scoped.{MATCH} = 1 \
             GROUP BY below.__og_t",
            table = quote(compiler.table)?
        ));
        let mut prepared = connection.prepare(&sql.sql).map_err(sqlite)?;
        let mut cursor = prepared
            .query(rusqlite::params_from_iter(sql.params.iter()))
            .map_err(sqlite)?;
        while let Some(row) = cursor.next().map_err(sqlite)? {
            let top: i64 = row.get(0).map_err(sqlite)?;
            let mut values = Vec::with_capacity(fields.len());
            for (index, field) in fields.iter().enumerate() {
                let cell: Sql = row.get(index + 1).map_err(sqlite)?;
                values.push(read(cell, field.data_type, field.name.as_str())?);
            }
            by_row.insert(top, values);
        }
    }
    // Column by column, in the page's order; a node with no match below it
    // counts none and has no other aggregate (S11).
    Ok((0..fields.len())
        .map(|index| {
            rows.iter()
                .map(|row| match by_row.get(row) {
                    Some(values) => values[index].clone(),
                    None if tree.aggregate[index].function
                        == opengrid_query::AggregateFn::Count =>
                    {
                        Value::Int64(0)
                    }
                    None => Value::Null,
                })
                .collect()
        })
        .collect())
}

/// The level of a node that does not exist: no rows, the tree's counts.
fn empty(query: &ValidatedQuery, tree: &ValidatedTree, matches: u64, orphans: u64) -> QueryResult {
    let mut result = QueryResult::new(
        query.output_schema.clone(),
        vec![Vec::new(); query.output_schema.len()],
        0,
    );
    result.tree = Some(TreeLevel {
        matches,
        orphans,
        aggregate_schema: if tree.aggregate.is_empty() {
            Default::default()
        } else {
            tree.aggregate_schema.clone()
        },
        aggregates: vec![Vec::new(); tree.aggregate.len()],
        ..TreeLevel::default()
    });
    result
}
