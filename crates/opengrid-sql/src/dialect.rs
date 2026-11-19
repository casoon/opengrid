//! What differs between databases.

use opengrid_query::{CmpOp, NullsOrder};
use opengrid_types::{DataType, DatePart, Value};

use crate::CompileError;

/// The statement under construction.
///
/// The only way to place a value is [`Sql::param`], and it writes a placeholder.
/// That is what makes "no value in the SQL text" a property of the type rather
/// than a habit.
pub struct Sql<'d> {
    dialect: &'d dyn Dialect,
    text: String,
    params: Vec<Value>,
}

impl<'d> Sql<'d> {
    pub(crate) fn new(dialect: &'d dyn Dialect) -> Self {
        Self {
            dialect,
            text: String::new(),
            params: Vec::new(),
        }
    }

    /// Appends SQL text. Never a value — see [`Sql::param`].
    pub fn push(&mut self, text: &str) {
        self.text.push_str(text);
    }

    /// Appends a placeholder **with its type** and remembers the value for it
    /// (the dialect may bring the value into the form the database stores).
    pub fn param(&mut self, value: &Value, data_type: DataType) -> Result<(), CompileError> {
        let stored = self.dialect.param_value(value, data_type)?;
        self.params.push(stored);
        let placeholder = self.dialect.placeholder(self.params.len(), data_type);
        self.text.push_str(&placeholder);
        Ok(())
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn finish(self) -> crate::CompiledQuery {
        crate::CompiledQuery {
            sql: self.text,
            params: self.params,
        }
    }
}

/// Quotes an identifier, refusing the one character that could end the quoting.
///
/// Identifiers only ever come from the schema or the configuration, so this can
/// never be reached by a request — it is the second lock on a door that should
/// already be closed.
pub fn quote_ident(name: &str) -> Result<String, CompileError> {
    if name.is_empty() || name.contains('"') || name.contains('\0') {
        return Err(CompileError::Identifier {
            name: name.to_owned(),
        });
    }
    Ok(format!("\"{name}\""))
}

/// Everything the compiler cannot say the same way for every database.
///
/// What a dialect cannot do is a capability ([`Dialect::grouping_sets`]): the
/// compiler refuses to write it and the engine answers that part as it does for
/// any source.
pub trait Dialect {
    /// The placeholder for the `number`-th parameter (1-based), with whatever
    /// cast the database needs to read `data_type` from it.
    fn placeholder(&self, number: usize, data_type: DataType) -> String;

    /// The value as it travels to the database: a dialect that stores decimals
    /// as scaled integers brings them to the column's scale here, and refuses
    /// one that does not fit.
    fn param_value(&self, value: &Value, _data_type: DataType) -> Result<Value, CompileError> {
        Ok(value.clone())
    }

    /// The column as a **comparison and sort operand**: binary collation for a
    /// string where the database's default is not (S4).
    fn collate(&self, column: &str, data_type: DataType) -> String;

    /// A sort key under the collation of a locale (S4a); the tag is letters,
    /// digits and `-`, so it cannot leave the quotes.
    fn locale_sort(&self, column: &str, locale: &str) -> String;

    /// ` NULLS FIRST` or ` NULLS LAST`, written out (S3).
    fn nulls(&self, nulls: NullsOrder) -> &'static str {
        match nulls {
            NullsOrder::First => " NULLS FIRST",
            NullsOrder::Last => " NULLS LAST",
        }
    }

    /// One `ORDER BY` term of a sort key: the key under its collation (S4, or
    /// a locale's, S4a), the direction, and where the NULLs go (S3). A
    /// database without `NULLS FIRST`/`LAST`, or whose locale collations are
    /// not deterministic, writes more than one term here.
    fn order(
        &self,
        column: &str,
        data_type: DataType,
        locale: Option<&str>,
        descending: bool,
        nulls: NullsOrder,
    ) -> Result<String, CompileError> {
        let key = match locale {
            Some(locale) => self.locale_sort(column, locale),
            None => self.collate(column, data_type),
        };
        let direction = if descending { " DESC" } else { " ASC" };
        Ok(format!("{key}{direction}{}", self.nulls(nulls)))
    }

    /// A derived column: the year or month of `source`, in UTC (S9).
    fn date_part(&self, part: DatePart, source: &str, source_type: Option<DataType>) -> String;

    /// One aggregate over `column`, answering in the types rule S12 names.
    fn aggregate(
        &self,
        function: opengrid_query::AggregateFn,
        column: &str,
        data_type: DataType,
    ) -> String;

    /// The constant true and false, which an empty `and` and `or` are.
    fn true_literal(&self) -> &'static str;
    fn false_literal(&self) -> &'static str;

    /// `1` in the type `count(*)` has, for the count of an aggregate that is one row.
    fn one_row(&self) -> &'static str;

    /// `contains`, `starts_with`, `icontains` and `istarts_with` (S5, S5a).
    fn text_match(
        &self,
        op: CmpOp,
        column: &str,
        value: &Value,
        data_type: DataType,
        sql: &mut Sql<'_>,
    ) -> Result<(), CompileError>;

    /// `has`: the list `column` holds `value` (S1: unknown for a NULL list).
    fn has(
        &self,
        column: &str,
        value: &Value,
        element: DataType,
        sql: &mut Sql<'_>,
    ) -> Result<(), CompileError>;

    /// `has_all`: the list holds every value (and a NULL list is unknown).
    fn has_all(
        &self,
        column: &str,
        values: &[Value],
        element: DataType,
        sql: &mut Sql<'_>,
    ) -> Result<(), CompileError>;

    /// `is_empty`: a list that is there and holds nothing.
    fn is_empty_list(&self, column: &str) -> String;

    /// The paging clause, parameters and all.
    fn paging(
        &self,
        limit: Option<u64>,
        offset: Option<u64>,
        sql: &mut Sql<'_>,
    ) -> Result<(), CompileError>;

    /// A pivot as one statement needs `GROUP BY GROUPING SETS` and `GROUPING()`.
    fn grouping_sets(&self) -> bool;

    /// A tree's level as SQL needs `WITH RECURSIVE`.
    fn recursive(&self) -> bool {
        true
    }

    // ---- A tree's level (issue #132) ------------------------------------

    /// The row's identity in the table: its `ctid` or `rowid`.
    fn row_id(&self) -> &'static str;

    /// The temporary table a tree's statements read: the statement that makes
    /// it up to `AS SELECT`, and the name the statements read it by.
    fn tree_create(&self) -> &'static str;
    fn tree_table(&self) -> &'static str;

    /// The statements that index the temporary table.
    fn tree_indexes(&self) -> &'static str;

    /// `filter` as the value of the match column: its opening and closing text
    /// around the filter, and the text when there is no filter.
    fn tree_match(&self) -> (&'static str, &'static str, &'static str);

    /// The match column as a condition.
    fn is_match(&self, column: &str) -> String;

    /// A count as the form the connector reads it back in.
    fn read_count(&self, expression: &str) -> String;

    /// A column in the form the connector reads it back in.
    fn read_column(&self, column: &str, data_type: DataType) -> String;
}
