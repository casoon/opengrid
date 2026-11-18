//! One level of a tree as SQL (issue #132, rules T1–T7).
//!
//! The tree's rows — each one's row id as its identity (a NULL key cannot be
//! one), key, parent and whether it matches the filter — are copied once into a
//! temporary table under the tree's scope ([`Compiler::tree_prepare`]); every
//! statement then reads it as `scoped`, and the source's table only for the
//! columns a level shows. `UNION` in every recursion, so a cycle ends the walk;
//! T3 names it.

use opengrid_query::{ValidatedQuery, ValidatedTree};
use opengrid_types::DataType;

use crate::dialect::{Dialect, Sql, quote_ident};
use crate::{CompileError, CompiledQuery, Compiler};

/// The columns a tree's statements carry beside the table's own.
const KEY: &str = "__og_key";
const PARENT: &str = "__og_parent";
const MATCH: &str = "__og_match";
const ROW: &str = "__og_row";

/// The extra output columns of a tree's level: each node's shown children and
/// whether it matches.
pub const TREE_CHILDREN: &str = "__og_children";
pub const TREE_MATCHED: &str = "__og_matched";
const POSITION: &str = "__og_position";

/// The source's own table, joined by row id for the columns a level shows.
const BASE: &str = "og_base";

impl<D: Dialect> Compiler<D> {
    fn recursive(&self) -> Result<(), CompileError> {
        if self.dialect.recursive() {
            Ok(())
        } else {
            Err(CompileError::Unsupported {
                what: "WITH RECURSIVE",
            })
        }
    }

    /// The tree's rows as a temporary table: the statement that makes it. The
    /// connector indexes it afterwards (`Dialect::tree_indexes`).
    pub fn tree_prepare(
        &self,
        query: &ValidatedQuery,
        tree: &ValidatedTree,
    ) -> Result<CompiledQuery, CompileError> {
        self.recursive()?;
        let mut sql = self.sql();
        sql.push(&format!(
            "{} {} AS {ROW}, {} AS {KEY}, {} AS {PARENT}, ",
            self.dialect.tree_create(),
            self.dialect.row_id(),
            self.column(tree.key.as_str())?,
            self.column(tree.parent.as_str())?
        ));
        let (open, close, none) = self.dialect.tree_match();
        match &query.filter {
            Some(filter) => {
                sql.push(open);
                self.filter(filter, &mut sql)?;
                sql.push(close);
            }
            None => sql.push(none),
        }
        sql.push(&format!(" AS {MATCH} FROM "));
        sql.push(&self.table_ref()?);
        if let Some(scope) = &tree.scope {
            sql.push(" WHERE ");
            self.filter(scope, &mut sql)?;
        }
        Ok(sql.finish())
    }

    /// `WITH RECURSIVE scoped AS (…)`: the tree's rows as prepared —
    /// `NOT MATERIALIZED`, or PostgreSQL copies a CTE named more than once into
    /// a table of its own, without the indexes, and sorts the whole tree in
    /// every step of a recursion (12 s instead of 0.1 s at 100 000 nodes).
    fn scoped(&self, sql: &mut Sql<'_>) {
        sql.push(&format!(
            "WITH RECURSIVE scoped AS NOT MATERIALIZED (SELECT * FROM {})",
            self.dialect.tree_table()
        ));
    }

    /// The key of a row, in the form the connector reads it back in.
    fn tree_key_text(&self, tree: &ValidatedTree) -> Result<String, CompileError> {
        Ok(self
            .dialect
            .read_column(KEY, self.data_type(tree.key.as_str())?))
    }

    /// A root's condition on the row `alias` (T2): no parent, or one that is no node.
    fn tree_root(alias: &str) -> String {
        format!(
            "({alias}.{PARENT} IS NULL OR NOT EXISTS \
             (SELECT 1 FROM scoped p WHERE p.{KEY} = {alias}.{PARENT}))"
        )
    }

    /// T1: one key that appears more than once — none if every key is there once.
    pub fn tree_duplicate_key(&self, tree: &ValidatedTree) -> Result<CompiledQuery, CompileError> {
        self.recursive()?;
        let mut sql = self.sql();
        self.scoped(&mut sql);
        sql.push(&format!(
            " SELECT {} FROM scoped WHERE {KEY} IS NOT NULL GROUP BY {KEY} \
             HAVING count(*) > 1 ORDER BY {KEY} LIMIT 1",
            self.tree_key_text(tree)?
        ));
        Ok(sql.finish())
    }

    /// T3: a node of a cycle — none if every node is reached from a root. A node
    /// not reached lies on a cycle or below one; the walk up from it finds the
    /// one that is its own ancestor.
    pub fn tree_cycle(&self, tree: &ValidatedTree) -> Result<CompiledQuery, CompileError> {
        self.recursive()?;
        let mut sql = self.sql();
        self.scoped(&mut sql);
        sql.push(&format!(
            ", reached(__og_r) AS (SELECT c.{ROW} FROM scoped c WHERE {root} \
             UNION SELECT c.{ROW} FROM reached JOIN scoped p ON p.{ROW} = reached.__og_r \
             JOIN scoped c ON c.{PARENT} = p.{KEY}), \
             lost(__og_r) AS (SELECT s.{ROW} FROM scoped s WHERE NOT EXISTS \
             (SELECT 1 FROM reached WHERE reached.__og_r = s.{ROW})), \
             up(__og_s, __og_r, __og_d) AS (SELECT __og_r, __og_r, 0 FROM lost \
             UNION SELECT up.__og_s, p.{ROW}, up.__og_d + 1 FROM up JOIN scoped c ON c.{ROW} = up.__og_r \
             JOIN scoped p ON p.{KEY} = c.{PARENT} WHERE up.__og_d < (SELECT count(*) FROM lost)) \
             SELECT {key} FROM up JOIN scoped ON scoped.{ROW} = up.__og_s \
             WHERE up.__og_r = up.__og_s AND up.__og_d > 0 ORDER BY up.__og_s LIMIT 1",
            root = Self::tree_root("c"),
            key = self.tree_key_text(tree)?,
        ));
        Ok(sql.finish())
    }

    /// T2 and T5: the orphans and the matches of the whole tree, and — when
    /// the query names a node — whether that node exists (T4).
    pub fn tree_counts(&self, tree: &ValidatedTree) -> Result<CompiledQuery, CompileError> {
        self.recursive()?;
        // The one parameter is the last thing in the statement, so the part
        // that holds it is compiled by itself and its text wrapped.
        let mut under = self.sql();
        let exists = match &tree.under {
            Some(value) => {
                under.push(&format!("SELECT count(*) FROM scoped WHERE {KEY} = "));
                under.param(value, self.data_type(tree.key.as_str())?)?;
                let counted = under.finish();
                (self.dialect.read_count(&counted.sql), counted.params)
            }
            None => (self.dialect.read_count("1"), Vec::new()),
        };
        let mut head = self.sql();
        self.scoped(&mut head);
        let head = head.finish();
        Ok(CompiledQuery {
            sql: format!(
                "{} SELECT {}, {}, {}",
                head.sql,
                self.dialect.read_count(&format!(
                    "SELECT count(*) FROM scoped c WHERE c.{PARENT} IS NOT NULL AND NOT EXISTS \
                     (SELECT 1 FROM scoped p WHERE p.{KEY} = c.{PARENT})"
                )),
                self.dialect.read_count(&format!(
                    "SELECT count(*) FROM scoped WHERE {}",
                    self.dialect.is_match(MATCH)
                )),
                exists.0
            ),
            params: exists.1,
        })
    }

    /// `, shown(__og_r), members(__og_r)`: the matches with their ancestors (T5), and of
    /// those the children of `under`, or the roots (T4).
    fn tree_members(&self, tree: &ValidatedTree, sql: &mut Sql<'_>) -> Result<(), CompileError> {
        sql.push(&format!(
            ", shown(__og_r) AS (SELECT {ROW} FROM scoped WHERE {matched} \
             UNION SELECT p.{ROW} FROM shown JOIN scoped c ON c.{ROW} = shown.__og_r \
             JOIN scoped p ON p.{KEY} = c.{PARENT}), \
             members(__og_r) AS (SELECT c.{ROW} FROM scoped c JOIN shown ON shown.__og_r = c.{ROW} WHERE ",
            matched = self.dialect.is_match(MATCH),
        ));
        match &tree.under {
            Some(under) => {
                sql.push(&format!("c.{PARENT} = "));
                sql.param(under, self.data_type(tree.key.as_str())?)?;
            }
            None => sql.push(&Self::tree_root("c")),
        }
        sql.push(")");
        Ok(())
    }

    /// How many nodes the level has, before paging (T6).
    pub fn tree_total(&self, tree: &ValidatedTree) -> Result<CompiledQuery, CompileError> {
        self.recursive()?;
        let mut sql = self.sql();
        self.scoped(&mut sql);
        self.tree_members(tree, &mut sql)?;
        sql.push(&format!(
            " SELECT {} FROM members",
            self.dialect.read_count("count(*)")
        ));
        Ok(sql.finish())
    }

    /// The level itself: the query's columns, each node's shown children and
    /// whether it matches, sorted among siblings (T6, ties by the key) and
    /// paged — and, asked for, each node's subtree aggregates over the
    /// matches below and at it (T7). Answers the statement and its output
    /// columns, every one in the form the dialect reads back.
    pub fn tree_level(
        &self,
        query: &ValidatedQuery,
        tree: &ValidatedTree,
    ) -> Result<(CompiledQuery, Vec<(String, DataType)>), CompileError> {
        self.recursive()?;
        let mut sql = self.sql();
        self.scoped(&mut sql);
        self.tree_members(tree, &mut sql)?;
        sql.push(&format!(
            ", kids(__og_r, __og_n) AS (SELECT p.{ROW}, count(*) FROM shown JOIN scoped c ON c.{ROW} = shown.__og_r \
             JOIN scoped p ON p.{KEY} = c.{PARENT} GROUP BY p.{ROW}), \
             level AS (SELECT "
        ));
        for field in query.output_schema.fields() {
            sql.push(&self.column(field.name.as_str())?);
            sql.push(" AS ");
            sql.push(&quote_ident(field.name.as_str())?);
            sql.push(", ");
        }
        let row_id = self.dialect.row_id();
        sql.push(&format!(
            "coalesce(kids.__og_n, 0) AS {TREE_CHILDREN}, scoped.{MATCH} AS {TREE_MATCHED}, \
             scoped.{KEY} AS {KEY}, scoped.{ROW} AS {ROW} FROM members \
             JOIN scoped ON scoped.{ROW} = members.__og_r \
             JOIN {base} AS {BASE} ON {BASE}.{row_id} = scoped.{ROW} LEFT JOIN kids ON kids.__og_r = scoped.{ROW}), \
             page AS MATERIALIZED (SELECT *, row_number() OVER (ORDER BY ",
            base = self.table_ref()?
        ));
        // The order once for the numbering and once for the paging: an output
        // column is an input column of `level`, so its collation can be written.
        let mut order = self.sql();
        for key in &query.sort {
            self.sort_key(key, query, &mut order)?;
            order.push(", ");
        }
        let key_type = self.data_type(tree.key.as_str())?;
        order.push(&self.dialect.collate(KEY, key_type));
        order.push(" ASC NULLS LAST");
        sql.push(order.text());
        sql.push(&format!(") AS {POSITION} FROM level ORDER BY "));
        sql.push(order.text());
        self.dialect.paging(query.limit, query.offset, &mut sql)?;
        sql.push(")");

        let mut output: Vec<(String, DataType)> = query
            .output_schema
            .fields()
            .iter()
            .map(|field| (field.name.as_str().to_owned(), field.data_type))
            .collect();
        output.push((TREE_CHILDREN.to_owned(), DataType::Int64));
        output.push((TREE_MATCHED.to_owned(), DataType::Bool));

        if !tree.aggregate.is_empty() {
            sql.push(&format!(
                ", below(__og_t, __og_r) AS (SELECT {ROW}, {ROW} FROM page \
                 UNION SELECT below.__og_t, c.{ROW} FROM below JOIN scoped p ON p.{ROW} = below.__og_r \
                 JOIN scoped c ON c.{PARENT} = p.{KEY}), \
                 subtree AS (SELECT below.__og_t AS __og_top"
            ));
            for aggregate in &tree.aggregate {
                sql.push(", ");
                sql.push(&self.aggregate(aggregate)?);
                sql.push(" AS ");
                sql.push(&quote_ident(aggregate.alias.as_str())?);
            }
            sql.push(&format!(
                " FROM below JOIN scoped ON scoped.{ROW} = below.__og_r \
                 JOIN {base} AS {BASE} ON {BASE}.{row_id} = scoped.{ROW} \
                 WHERE {matched} GROUP BY below.__og_t)",
                base = self.table_ref()?,
                matched = self.dialect.is_match(&format!("scoped.{MATCH}")),
            ));
            for field in tree.aggregate_schema.fields() {
                output.push((field.name.as_str().to_owned(), field.data_type));
            }
        }

        // Every column read back in the dialect's form, the page in its order.
        sql.push(" SELECT ");
        for (index, (name, data_type)) in output.iter().enumerate() {
            if index > 0 {
                sql.push(", ");
            }
            sql.push(&self.dialect.read_column(&quote_ident(name)?, *data_type));
        }
        sql.push(" FROM page");
        if !tree.aggregate.is_empty() {
            sql.push(&format!(
                " LEFT JOIN subtree ON subtree.__og_top = page.{ROW}"
            ));
        }
        sql.push(&format!(" ORDER BY page.{POSITION}"));
        Ok((sql.finish(), output))
    }
}
