//! The rows tier (issue #46, E36): a source that can only hand out rows.
//!
//! A file, a list in memory, a service without a query language — anything
//! that can say which columns it has and hand out its rows is a source.
//! [`Rows`] turns it into a full [`Connector`]: `opengrid-engine` answers the
//! query — filter, sort, group, aggregate, paging, `total_count` — natively,
//! on the server.
//!
//! # Streaming (issue #50)
//!
//! The rows go through piece by piece, and the server keeps only what the
//! answer needs — memory follows the answer, not the source:
//!
//! - **A page** (`limit`, with or without `sort`): the best `offset + limit`
//!   rows so far, cut back whenever the held rows grow, and a counter for
//!   `total_count`. The engine's sort is stable and the pieces keep their
//!   order, so the page is the one a sort of the whole table would give.
//! - **Groups and aggregates**: per piece the partial aggregates — `sum`,
//!   `count`, `min`, `max`, and `avg` as a sum and a count — merged as they
//!   come; the averages are divided at the end.
//! - **Every match** (no `limit`, no grouping — an export, say): the matching
//!   rows, which is what that answer is.
//!
//! [`Rows::max_scan_rows`] bounds the rows (or groups) held for one answer:
//! more is an error that says so ([`DataSourceError::LimitExceeded`]), not a
//! server out of memory. How long a scan may take is the server's timeout.
//!
//! # The filter is a hint
//!
//! [`RowSource::scan`] gets the query's filter — the mandatory row filter
//! already in it. A source may use it to hand out fewer rows; it does not have
//! to. The engine applies the filter again either way, so a source that
//! ignores it, or applies it wrongly, cannot let a row through that the filter
//! excludes.

use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;
use std::task::{Context, Poll};
use std::time::Duration;

use opengrid_columns::Table;
use opengrid_engine::execute::execute as run;
use opengrid_engine::ingest::PieceBuilder;
use opengrid_query::{Aggregate, AggregateFn, Limits, Query, ValidatedFilter};
use opengrid_types::{FieldName, Value};

use crate::{
    BoxFuture, Connector, DataSourceCapabilities, DataSourceError, ExportRows, QueryResult, Schema,
    ValidatedQuery,
};

/// What a source of the rows tier implements.
pub trait RowSource: Send + Sync {
    /// Every column: the stored ones, and derived ones (`"from": { "part":
    /// "year", … }`), which the server computes.
    fn schema(&self) -> BoxFuture<'_, Result<Schema, DataSourceError>>;

    /// Hands out the rows, a piece at a time. Each piece holds the **stored**
    /// columns of the schema, in order. `filter` is a hint (module docs).
    fn scan<'a>(
        &'a self,
        filter: Option<&'a ValidatedFilter>,
    ) -> BoxFuture<'a, Result<Box<dyn RowStream + 'a>, DataSourceError>>;
}

/// The rows of one scan.
pub trait RowStream: Send {
    /// The next piece, or `None` after the last one.
    fn next_piece(&mut self) -> BoxFuture<'_, Result<Option<QueryResult>, DataSourceError>>;
}

/// A [`RowSource`] as a [`Connector`], answered by the engine.
pub struct Rows<R> {
    source: R,
    max_scan_rows: u64,
    compact_at: usize,
    schema: OnceLock<Schema>,
}

impl<R: RowSource> Rows<R> {
    /// One million rows or groups held for one answer: about 300 MiB for ten
    /// columns. A page or a grouping holds far less, whatever the source's size.
    pub const DEFAULT_MAX_SCAN_ROWS: u64 = 1_000_000;

    /// Rows held before they are cut back to what the answer needs.
    pub const DEFAULT_COMPACT_AT: usize = 65_536;

    pub fn new(source: R) -> Self {
        Self {
            source,
            max_scan_rows: Self::DEFAULT_MAX_SCAN_ROWS,
            compact_at: Self::DEFAULT_COMPACT_AT,
            schema: OnceLock::new(),
        }
    }

    /// How many rows (or partial groups) pile up before they are cut back to
    /// what the answer needs: lower holds less, higher sorts and merges less
    /// often. The answer is the same either way.
    pub fn compact_at(mut self, rows: usize) -> Self {
        self.compact_at = rows.max(1);
        self
    }

    /// The most rows (or groups) one answer may hold.
    pub fn max_scan_rows(mut self, rows: u64) -> Self {
        self.max_scan_rows = rows;
        self
    }

    /// The schema, asked once.
    async fn full_schema(&self) -> Result<Schema, DataSourceError> {
        if let Some(schema) = self.schema.get() {
            return Ok(schema.clone());
        }
        let schema = self.source.schema().await?;
        Ok(self.schema.get_or_init(|| schema).clone())
    }

    /// Answers `query` from the source's rows, keeping only what it needs.
    async fn stream(&self, query: &ValidatedQuery) -> Result<QueryResult, DataSourceError> {
        let schema = self.full_schema().await?;
        if query.tree.is_some() {
            return self.stream_tree(&schema, query).await;
        }
        if query.group.is_empty() && query.aggregate.is_empty() {
            self.stream_rows(&schema, query).await
        } else {
            self.stream_groups(&schema, query).await
        }
    }

    fn check_held(&self, held: usize, what: &str) -> Result<(), DataSourceError> {
        if held as u64 > self.max_scan_rows {
            return Err(DataSourceError::LimitExceeded {
                message: format!(
                    "more than {} {what} to hold for this answer (max_scan_rows); \
                     narrow the filter or ask for a page",
                    self.max_scan_rows
                ),
            });
        }
        Ok(())
    }

    /// A query without grouping: the best `offset + limit` rows — every match
    /// without a `limit` — and the count of all matches.
    async fn stream_rows(
        &self,
        schema: &Schema,
        query: &ValidatedQuery,
    ) -> Result<QueryResult, DataSourceError> {
        let table_schema = schema.materialized();
        let keep = query
            .limit
            .map(|limit| limit.saturating_add(query.offset.unwrap_or(0)));
        // Every column, sorted as the query sorts, cut to what may still make
        // the page.
        let window = ValidatedQuery {
            select: table_schema
                .fields()
                .iter()
                .map(|field| field.name.clone())
                .collect(),
            filter: None,
            group: Vec::new(),
            aggregate: Vec::new(),
            sort: query.sort.clone(),
            offset: None,
            limit: keep,
            output_schema: table_schema.clone(),
            source: query.source.clone(),
            tree: None,
        };
        let filtering = ValidatedQuery {
            filter: query.filter.clone(),
            ..window.clone()
        };
        let compact_at = keep.map(|keep| {
            self.compact_at.max(
                usize::try_from(keep)
                    .unwrap_or(usize::MAX)
                    .saturating_mul(2),
            )
        });

        let mut held: Vec<Table> = Vec::new();
        let mut held_rows = 0usize;
        let mut matched = 0u64;
        let mut stream = self.source.scan(query.filter.as_ref()).await?;
        while let Some(piece) = stream.next_piece().await? {
            yield_now().await;
            let kept = run(&table_of(schema, &piece)?, &filtering).map_err(engine)?;
            matched += kept.total_count;
            held_rows += kept.table.num_rows();
            held.push(kept.table);
            // Cut back when the held rows grow — and before the bound
            // judges them: a page never has to hold more than itself.
            if compact_at.is_some_and(|at| held_rows > at || held_rows as u64 > self.max_scan_rows)
            {
                let all = Table::concat(&table_schema, &held).map_err(backend)?;
                let cut = run(&all, &window).map_err(engine)?.table;
                held_rows = cut.num_rows();
                held = vec![cut];
            }
            self.check_held(held_rows, "matching rows")?;
        }

        let all = Table::concat(&table_schema, &held).map_err(backend)?;
        let answer = ValidatedQuery {
            filter: None,
            ..query.clone()
        };
        let page = run(&all, &answer).map_err(engine)?;
        Ok(QueryResult::new(
            query.output_schema.clone(),
            page.table.to_values(),
            matched,
        ))
    }

    /// One level of a tree (E38): every row of the tree's scope is held — a
    /// level and its counts depend on all of them (T4, T5) — and the engine
    /// answers the level once they are in. The scope is the source's hint,
    /// not the query's filter: the ancestors of T5 do not pass that.
    async fn stream_tree(
        &self,
        schema: &Schema,
        query: &ValidatedQuery,
    ) -> Result<QueryResult, DataSourceError> {
        let table_schema = schema.materialized();
        let scope = query.tree.as_ref().and_then(|tree| tree.scope.as_ref());
        let mut held: Vec<Table> = Vec::new();
        let mut held_rows = 0usize;
        let mut stream = self.source.scan(scope).await?;
        while let Some(piece) = stream.next_piece().await? {
            yield_now().await;
            let table = table_of(schema, &piece)?;
            held_rows += table.num_rows();
            held.push(table);
            self.check_held(held_rows, "rows of the tree")?;
        }
        let all = Table::concat(&table_schema, &held).map_err(backend)?;
        let level = run(&all, query).map_err(engine)?;
        let mut answer = QueryResult::new(
            query.output_schema.clone(),
            level.table.to_values(),
            level.total_count,
        );
        answer.tree = level.tree;
        Ok(answer)
    }

    /// A query with grouping or aggregates: partial aggregates per piece,
    /// merged as they come.
    async fn stream_groups(
        &self,
        schema: &Schema,
        query: &ValidatedQuery,
    ) -> Result<QueryResult, DataSourceError> {
        let table_schema = schema.materialized();
        let plan = Partials::of(query);
        let partial = plan.partial_query(query, &table_schema)?;
        let merge = plan.merge_query(query, &partial.output_schema)?;

        let mut held: Vec<Table> = Vec::new();
        let mut held_rows = 0usize;
        let mut stream = self.source.scan(query.filter.as_ref()).await?;
        while let Some(piece) = stream.next_piece().await? {
            yield_now().await;
            let part = run(&table_of(schema, &piece)?, &partial)
                .map_err(engine)?
                .table;
            held_rows += part.num_rows();
            held.push(part);
            if held_rows > self.compact_at || held_rows as u64 > self.max_scan_rows {
                let all = Table::concat(&partial.output_schema, &held).map_err(backend)?;
                let merged = run(&all, &merge).map_err(engine)?.table;
                held_rows = merged.num_rows();
                held = vec![merged];
                self.check_held(held_rows, "groups")?;
            }
        }
        if held.is_empty() {
            // Rule S11: an aggregate without groups answers one row even over
            // no rows at all — the engine says what that row holds.
            let empty = PieceBuilder::new(schema).finish().map_err(ingest)?;
            held.push(run(&empty, &partial).map_err(engine)?.table);
        }
        let all = Table::concat(&partial.output_schema, &held).map_err(backend)?;
        let merged = run(&all, &merge).map_err(engine)?.table;
        self.check_held(merged.num_rows(), "groups")?;

        let columns = plan.finish(query, &merged)?;
        let groups = Table::from_values(&query.output_schema, &columns).map_err(backend)?;
        let answer = ValidatedQuery {
            select: query
                .output_schema
                .fields()
                .iter()
                .map(|field| field.name.clone())
                .collect(),
            filter: None,
            group: Vec::new(),
            aggregate: Vec::new(),
            ..query.clone()
        };
        let page = run(&groups, &answer).map_err(engine)?;
        Ok(QueryResult::new(
            query.output_schema.clone(),
            page.table.to_values(),
            page.total_count,
        ))
    }
}

impl<R: RowSource> Connector for Rows<R> {
    fn schema(&self) -> BoxFuture<'_, Result<Schema, DataSourceError>> {
        Box::pin(self.full_schema())
    }

    /// The engine answers everything once the rows arrive.
    fn capabilities(&self) -> DataSourceCapabilities {
        DataSourceCapabilities::ALL
    }

    fn execute(
        &self,
        query: ValidatedQuery,
    ) -> BoxFuture<'_, Result<QueryResult, DataSourceError>> {
        Box::pin(async move { self.stream(&query).await })
    }

    /// Scans once and hands out slices of the one answer; the default would
    /// scan twice, once to count and once to read.
    fn export<'a>(
        &'a self,
        query: &'a ValidatedQuery,
        _idle_limit: Duration,
    ) -> BoxFuture<'a, Result<Box<dyn ExportRows + 'a>, DataSourceError>> {
        Box::pin(async move {
            let answer = self.stream(query).await?;
            let pieces: Box<dyn ExportRows + 'a> = Box::new(Answered { answer, next: 0 });
            Ok(pieces)
        })
    }
}

/// Gives the runtime its turn once. A source whose pieces are ready at once
/// (a file in memory, a generator) would otherwise never let the server's
/// timeout fire, nor another request run on this thread, for the whole scan.
/// Runtime-neutral: pending once, woken at once.
fn yield_now() -> impl Future<Output = ()> {
    struct YieldNow(bool);
    impl Future for YieldNow {
        type Output = ();
        fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<()> {
            if self.0 {
                return Poll::Ready(());
            }
            self.0 = true;
            context.waker().wake_by_ref();
            Poll::Pending
        }
    }
    YieldNow(false)
}

/// A piece as a table of the full schema, derived columns computed.
fn table_of(schema: &Schema, piece: &QueryResult) -> Result<Table, DataSourceError> {
    let mut builder = PieceBuilder::new(schema);
    builder.push(piece).map_err(ingest)?;
    builder.finish().map_err(ingest)
}

/// How one aggregate of a query is taken apart into mergeable pieces.
enum Part {
    /// `sum`, `min`, `max`, `count`: one partial, merged by summing or by
    /// its own function.
    One(FieldName),
    /// `avg`: a sum and a count, divided at the end.
    Average { sum: FieldName, count: FieldName },
}

struct Partials {
    partials: Vec<Aggregate>,
    parts: Vec<Part>,
}

impl Partials {
    fn of(query: &ValidatedQuery) -> Self {
        let name = |i: usize, what: &str| {
            FieldName::new(format!("__og_{i}_{what}")).expect("a valid identifier")
        };
        let mut partials = Vec::new();
        let mut parts = Vec::new();
        for (i, aggregate) in query.aggregate.iter().enumerate() {
            let partial = |function, alias: FieldName| Aggregate {
                field: aggregate.field.clone(),
                function,
                alias,
            };
            if aggregate.function == AggregateFn::Avg {
                let (sum, count) = (name(i, "sum"), name(i, "count"));
                partials.push(partial(AggregateFn::Sum, sum.clone()));
                partials.push(partial(AggregateFn::Count, count.clone()));
                parts.push(Part::Average { sum, count });
            } else {
                let alias = name(i, "part");
                partials.push(partial(aggregate.function, alias.clone()));
                parts.push(Part::One(alias));
            }
        }
        Self { partials, parts }
    }

    /// Per piece: the query's filter, its groups, the partial aggregates.
    fn partial_query(
        &self,
        query: &ValidatedQuery,
        schema: &Schema,
    ) -> Result<ValidatedQuery, DataSourceError> {
        let mut partial = grouped(query, self.partials.clone(), schema)?;
        partial.filter = query.filter.clone();
        Ok(partial)
    }

    /// Over partials: the same groups, each partial merged — counts summed,
    /// sums summed, minimums of minimums, maximums of maximums.
    fn merge_query(
        &self,
        query: &ValidatedQuery,
        partial_schema: &Schema,
    ) -> Result<ValidatedQuery, DataSourceError> {
        // Validation refuses an alias that names an input column, so the
        // merged columns are validated under stand-in names and then given
        // the partials' names back — merged groups are merged again with the
        // next pieces, so they must look like partials.
        let stand_in =
            |j: usize| FieldName::new(format!("__og_merged_{j}")).expect("an identifier");
        let merges = self
            .partials
            .iter()
            .enumerate()
            .map(|(j, partial)| Aggregate {
                field: Some(partial.alias.clone()),
                function: match partial.function {
                    AggregateFn::Count => AggregateFn::Sum,
                    other => other,
                },
                alias: stand_in(j),
            })
            .collect();
        let mut merge = grouped(query, merges, partial_schema)?;
        for (j, partial) in self.partials.iter().enumerate() {
            merge.aggregate[j].alias = partial.alias.clone();
        }
        let fields = merge
            .output_schema
            .fields()
            .iter()
            .map(|field| {
                let mut field = field.clone();
                if let Some(j) = (0..self.partials.len()).find(|j| field.name == stand_in(*j)) {
                    field.name = self.partials[j].alias.clone();
                }
                field
            })
            .collect();
        merge.output_schema = Schema::new(fields);
        Ok(merge)
    }

    /// The query's output columns from the merged groups.
    fn finish(
        &self,
        query: &ValidatedQuery,
        merged: &Table,
    ) -> Result<Vec<Vec<Value>>, DataSourceError> {
        let rows = merged.num_rows();
        let column = |name: &str| -> Result<Vec<Value>, DataSourceError> {
            let column = merged
                .column(name)
                .ok_or_else(|| backend(format!("the merged groups have no column {name:?}")))?;
            Ok((0..rows).map(|row| column.value(row)).collect())
        };
        let mut columns = Vec::new();
        for field in query.output_schema.fields() {
            let name = field.name.as_str();
            let part = query
                .aggregate
                .iter()
                .position(|aggregate| aggregate.alias.as_str() == name)
                .map(|i| &self.parts[i]);
            columns.push(match part {
                None => column(name)?,
                Some(Part::One(alias)) => column(alias.as_str())?,
                Some(Part::Average { sum, count }) => column(sum.as_str())?
                    .iter()
                    .zip(&column(count.as_str())?)
                    .map(|(sum, count)| average(sum, count))
                    .collect(),
            });
        }
        Ok(columns)
    }
}

/// `sum / count` as rule S12 wants an average: a float, NULL over nothing.
fn average(sum: &Value, count: &Value) -> Value {
    let Value::Int64(count) = count else {
        return Value::Null;
    };
    if *count == 0 {
        return Value::Null;
    }
    let total = match sum {
        Value::Int64(sum) => *sum as f64,
        Value::Float64(sum) => *sum,
        Value::Decimal(sum) => sum.value() as f64 / 10f64.powi(i32::from(sum.scale())),
        _ => return Value::Null,
    };
    Value::Float64(total / *count as f64)
}

/// `query`'s groups with `aggregates`, validated against `schema` — the output
/// schema comes from the rules every query follows.
fn grouped(
    query: &ValidatedQuery,
    aggregates: Vec<Aggregate>,
    schema: &Schema,
) -> Result<ValidatedQuery, DataSourceError> {
    Query {
        source: query.source.clone(),
        select: query.group.clone(),
        filter: None,
        group: query.group.clone(),
        aggregate: aggregates,
        sort: Vec::new(),
        offset: None,
        limit: None,
        tree: None,
    }
    .validate(schema, &Limits::default())
    .map_err(|error| backend(format!("the partial aggregates: {error}")))
}

/// One answer, handed out in slices.
struct Answered {
    answer: QueryResult,
    next: usize,
}

impl ExportRows for Answered {
    fn count(&mut self) -> BoxFuture<'_, Result<u64, DataSourceError>> {
        let rows = self.answer.row_count() as u64;
        Box::pin(async move { Ok(rows) })
    }

    fn next_piece(&mut self, rows: usize) -> BoxFuture<'_, Result<QueryResult, DataSourceError>> {
        let start = self.next.min(self.answer.row_count());
        let end = (start + rows).min(self.answer.row_count());
        self.next = end;
        let columns = self
            .answer
            .columns
            .iter()
            .map(|column| column[start..end].to_vec())
            .collect();
        let piece = QueryResult::new(self.answer.schema.clone(), columns, self.answer.total_count);
        Box::pin(async move { Ok(piece) })
    }
}

fn ingest(error: opengrid_engine::ingest::IngestError) -> DataSourceError {
    backend(format!("the source's rows: {error}"))
}

fn engine(error: opengrid_engine::execute::ExecuteError) -> DataSourceError {
    backend(format!("the engine: {error}"))
}

fn backend(message: impl Into<String>) -> DataSourceError {
    DataSourceError::Backend {
        message: message.into(),
    }
}
