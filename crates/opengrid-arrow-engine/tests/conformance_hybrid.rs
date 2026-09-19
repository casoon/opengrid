//! The conformance suite over the **hybrid** path (plan point 28, step 7).
//!
//! Point 26 showed that a database answers the same thing as the engine. This
//! shows the third way of answering: a source that cannot do everything, with
//! the local engine finishing the rest — and the same 48 cases must still come
//! out identical, whichever capability is missing.
//!
//! The partial source is deliberately strict: it *refuses* an operation it does
//! not declare. A planner that pushed too much would not get a wrong answer
//! here, it would get an error — which makes this a test of the split, not just
//! of the engine behind it.

mod common;

use std::path::PathBuf;

use opengrid_arrow_engine::datasource::LocalDataSource;
use opengrid_arrow_engine::hybrid::HybridSource;
use opengrid_conformance::{RowOrder, Table, block_on, check_dir, compare};
use opengrid_datasource::{DataSourceCapabilities, DataSourceError, QueryResult, SendDataSource};
use opengrid_planner::ExecutionMode;
use opengrid_query::ValidatedQuery;
use opengrid_types::Schema;

fn cases_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../opengrid-conformance/cases")
}

/// A source that can do less than the engine — and insists on it.
struct Partial {
    inner: LocalDataSource,
    capabilities: DataSourceCapabilities,
}

impl Partial {
    fn new(capabilities: DataSourceCapabilities) -> Self {
        Self {
            inner: LocalDataSource::new(common::csv_batches()).expect("the dataset has batches"),
            capabilities,
        }
    }

    /// Everything the source was handed but never claimed to do.
    fn overreach(&self, query: &ValidatedQuery) -> Option<&'static str> {
        let checks = [
            (query.filter.is_some(), self.capabilities.filter, "filter"),
            (!query.group.is_empty(), self.capabilities.group, "group"),
            (
                !query.aggregate.is_empty(),
                self.capabilities.aggregate,
                "aggregate",
            ),
            (!query.sort.is_empty(), self.capabilities.sort, "sort"),
            (
                query.limit.is_some() || query.offset.is_some(),
                self.capabilities.paging,
                "page",
            ),
        ];
        checks
            .into_iter()
            .find(|(asked, can, _)| *asked && !*can)
            .map(|(_, _, name)| name)
    }
}

impl SendDataSource for Partial {
    async fn schema(&self) -> Result<Schema, DataSourceError> {
        Ok(common::schema())
    }

    async fn execute(&self, query: ValidatedQuery) -> Result<QueryResult, DataSourceError> {
        if let Some(operation) = self.overreach(&query) {
            return Err(DataSourceError::Backend {
                message: format!("this source was handed a {operation} it cannot do"),
            });
        }
        SendDataSource::execute(&self.inner, query).await
    }

    fn capabilities(&self) -> DataSourceCapabilities {
        self.capabilities
    }
}

/// Capability sets worth running the whole suite against, each named.
fn partial_sources() -> Vec<(&'static str, DataSourceCapabilities)> {
    let all = DataSourceCapabilities::ALL;
    let without = |name: &str| {
        let mut caps = all;
        match name {
            "filter" => caps.filter = false,
            "sort" => caps.sort = false,
            "group" => caps.group = false,
            "aggregate" => caps.aggregate = false,
            "paging" => caps.paging = false,
            other => panic!("unknown capability {other}"),
        }
        caps
    };
    vec![
        ("without filter", without("filter")),
        ("without sort", without("sort")),
        ("without group", without("group")),
        ("without aggregate", without("aggregate")),
        ("without paging", without("paging")),
        // The extreme: a source that only hands over rows. Every step is the
        // client's, which is what `mode="local"` looks like from here.
        ("rows only", DataSourceCapabilities::default()),
    ]
}

/// **The point of step 7:** 48/48 over the hybrid path, for every gap.
#[test]
fn the_hybrid_path_answers_the_whole_suite() {
    let schema = common::schema();
    let checked = check_dir(&cases_dir(), &schema).expect("the cases load");
    assert_eq!(checked.len(), 53, "the suite has 53 cases");

    let mut failed: Vec<String> = Vec::new();
    let mut split = 0usize;
    for (name, capabilities) in partial_sources() {
        let source = HybridSource::new(Partial::new(capabilities), schema.clone());
        for case in &checked {
            let order = if case.case.ordered {
                RowOrder::Ordered
            } else {
                RowOrder::Unordered
            };
            match block_on(source.run(case.query.clone())) {
                Ok((result, plan)) => {
                    if !plan.is_fully_pushed() {
                        split += 1;
                    }
                    if let Err(mismatch) = compare(&case.expected, &Table::from(&result), order) {
                        failed.push(format!("{name}, {}: {mismatch}", case.case.id));
                    }
                }
                Err(error) => failed.push(format!("{name}, {}: {error}", case.case.id)),
            }
        }
    }

    println!(
        "conformance (hybrid): {} cases × {} capability sets, {split} of them actually split, \
         {} failures",
        checked.len(),
        partial_sources().len(),
        failed.len()
    );
    assert!(
        failed.is_empty(),
        "{} runs differ:\n{}",
        failed.len(),
        failed.join("\n")
    );
    // A suite that never splits would prove nothing: most cases must have left
    // work for the client.
    assert!(split > 100, "too few split plans: {split}");
}

/// `total_count` is the count *after* the client's work, not the source's.
///
/// The grid shows it next to the page, so a hybrid run that reported the
/// unfiltered count would show a wrong number of rows.
#[test]
fn the_count_belongs_to_the_final_result() {
    use opengrid_query::{Limits, Query};

    let schema = common::schema();
    let query: Query = serde_json::from_str(
        r#"{"source":"orders","select":["id"],
            "filter":{"field":"country","op":"eq","value":"DE"},
            "sort":[{"field":"id","direction":"asc"}],
            "limit":2}"#,
    )
    .expect("a query");
    let validated = query.validate(&schema, &Limits::default()).expect("valid");

    let mut capabilities = DataSourceCapabilities::ALL;
    capabilities.filter = false;
    let source = HybridSource::new(Partial::new(capabilities), schema.clone());
    let (hybrid, plan) = block_on(source.run(validated.clone())).expect("the hybrid run");

    let local = LocalDataSource::new(common::csv_batches()).expect("batches");
    let here = block_on(SendDataSource::execute(&local, validated)).expect("the local run");

    // The sort *can* be pushed across a client filter — filtering keeps the
    // order — and the client query sorts again anyway before it pages.
    assert_eq!(plan.describe(), "source: sort | client: filter · page");
    assert_eq!(hybrid.total_count, here.total_count);
    assert_eq!(hybrid.row_count(), here.row_count());
    assert!(hybrid.total_count > hybrid.row_count() as u64);
}

/// The three explicit modes stay enforceable (plan point 28, step 5).
#[test]
fn the_explicit_modes_override_what_auto_would_do() {
    use opengrid_query::{Limits, Query};

    let schema = common::schema();
    let query: Query = serde_json::from_str(
        r#"{"source":"orders","select":["id"],
            "filter":{"field":"country","op":"eq","value":"DE"},
            "sort":[{"field":"id","direction":"asc"}],
            "limit":2}"#,
    )
    .expect("a query");
    let validated = query.validate(&schema, &Limits::default()).expect("valid");

    // A source that can do everything: `auto` pushes it all.
    let capable = HybridSource::new(Partial::new(DataSourceCapabilities::ALL), schema.clone());
    assert_eq!(
        capable.plan_for(&validated).unwrap().describe(),
        "source: filter · sort · page | client: —"
    );

    // `local` keeps it here anyway — and the answer stays the same.
    let local_mode = HybridSource::new(Partial::new(DataSourceCapabilities::ALL), schema.clone())
        .with_mode(ExecutionMode::Local);
    let (result, plan) = block_on(local_mode.run(validated.clone())).expect("a local run");
    assert_eq!(
        plan.describe(),
        "source: scan | client: filter · sort · page"
    );
    assert_eq!(result.row_count(), 2);

    // `remote` says what it cannot do instead of finishing it quietly.
    let mut capabilities = DataSourceCapabilities::ALL;
    capabilities.filter = false;
    let remote =
        HybridSource::new(Partial::new(capabilities), schema).with_mode(ExecutionMode::Remote);
    let error = remote.plan_for(&validated).expect_err("remote must refuse");
    assert!(
        error.to_string().contains("filter"),
        "the error must name the operation: {error}"
    );
}
