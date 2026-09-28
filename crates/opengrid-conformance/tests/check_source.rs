//! `check_source` (issue #47): the runner a connector author uses must pass a
//! source that is right and name the cases of one that is not.

use opengrid_conformance::{block_on, check_source, fixture_csv, fixture_schema};
use opengrid_datasource::{DataSourceCapabilities, DataSourceError, QueryResult, SendDataSource};
use opengrid_engine::datasource::LocalDataSource;
use opengrid_engine::ingest::{CsvOptions, load_csv};
use opengrid_query::ValidatedQuery;
use opengrid_types::Schema;

fn engine() -> LocalDataSource {
    let csv = std::fs::read(fixture_csv()).expect("the fixture");
    LocalDataSource::new(load_csv(&csv, &fixture_schema(), CsvOptions::default()).expect("a table"))
}

/// The engine, with every answer's rows in reverse — wrong wherever order counts.
struct Reversed(LocalDataSource);

impl SendDataSource for Reversed {
    fn schema(&self) -> impl Future<Output = Result<Schema, DataSourceError>> + Send {
        SendDataSource::schema(&self.0)
    }

    fn execute(
        &self,
        query: ValidatedQuery,
    ) -> impl Future<Output = Result<QueryResult, DataSourceError>> + Send {
        let answer = SendDataSource::execute(&self.0, query);
        async move {
            let mut result = answer.await?;
            for column in &mut result.columns {
                column.reverse();
            }
            Ok(result)
        }
    }

    fn capabilities(&self) -> DataSourceCapabilities {
        DataSourceCapabilities::ALL
    }
}

#[test]
fn the_engine_passes_and_a_wrong_source_is_named() {
    let right = block_on(check_source(&engine()));
    right.assert_ok();
    assert!(right.cases >= 40, "{right}");

    let wrong = block_on(check_source(&Reversed(engine())));
    assert!(!wrong.is_ok(), "reversed rows must differ somewhere");
    assert!(
        wrong.to_string().contains("of") && wrong.failures.iter().all(|line| line.contains(':')),
        "{wrong}"
    );
}
