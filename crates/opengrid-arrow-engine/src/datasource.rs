//! The local engine as a [`DataSource`](opengrid_datasource::DataSource).
//!
//! Point 09 binds data and engine in one place: [`LocalDataSource`] holds the
//! Arrow batches and answers with the Arrow-free
//! [`QueryResult`](opengrid_datasource::QueryResult) (E14). Before this point the
//! conformance runner needed a test-local adapter (`LocalEngine`), because the
//! orphan rule kept this crate from implementing the suite's trait — that
//! workaround is gone with the real trait.
//!
//! The source lives here rather than in `opengrid-datasource` because that crate
//! must stay free of Arrow (plan/spezifikation/11-crates.md §Portabilität, the
//! dependency points `arrow-engine → datasource`, never back).

use std::future::Future;

use arrow_array::RecordBatch;
use opengrid_datasource::{DataSourceCapabilities, DataSourceError, QueryResult, SendDataSource};
use opengrid_query::ValidatedQuery;
use opengrid_types::{DataType, Field, FieldName, Schema};

use crate::execute::{QueryResult as BatchResult, execute};
use crate::ingest::batch;

/// The in-memory source: Arrow batches plus the local executor.
///
/// This is the implementation the browser runs in WASM. It reports every
/// capability, because the engine answers filter, sort, group, aggregate and
/// paging itself (plan/spezifikation/03-datasource.md §Capabilities).
pub struct LocalDataSource {
    schema: Schema,
    batches: Vec<RecordBatch>,
}

impl LocalDataSource {
    /// Binds batches to the engine.
    ///
    /// The schema comes from the batches — ingest produced them against an
    /// explicit schema, so the data is the authority here. Batches without a
    /// single column or row of *schema* (an empty vector) carry none: that is
    /// [`DataSourceError::NoData`], not an empty table. An empty table is one
    /// batch with no rows, which ingest yields for an empty input.
    pub fn new(batches: Vec<RecordBatch>) -> Result<Self, DataSourceError> {
        let Some(first) = batches.first() else {
            return Err(DataSourceError::NoData);
        };
        Ok(Self {
            schema: schema_of(first)?,
            batches,
        })
    }
}

impl SendDataSource for LocalDataSource {
    /// The batches are `Send` and nothing is awaited, so the local source
    /// satisfies the server variant as well as the browser one (E5). The base
    /// `DataSource` comes from the macro's blanket impl.
    fn schema(&self) -> impl Future<Output = Result<Schema, DataSourceError>> + Send {
        let schema = self.schema.clone();
        async move { Ok(schema) }
    }

    fn execute(
        &self,
        query: ValidatedQuery,
    ) -> impl Future<Output = Result<QueryResult, DataSourceError>> + Send {
        let batches = &self.batches;
        async move {
            let BatchResult {
                schema,
                batches: result,
                total_count,
            } = execute(batches, &query).map_err(|error| DataSourceError::Backend {
                message: format!("local engine: {error}"),
            })?;

            // The executor returns exactly one batch
            // (plan/spezifikation/04-local-engine.md §Executor). A result without
            // it is a bug on our side, so it is reported instead of papered over
            // with an empty column set.
            let Some(batch) = result.first() else {
                return Err(DataSourceError::Backend {
                    message: "the local engine returned no batch".to_owned(),
                });
            };
            let columns = batch::decode(batch).map_err(|message| DataSourceError::Backend {
                message: format!("local engine: {message}"),
            })?;

            Ok(QueryResult::new(schema, columns, total_count))
        }
    }

    fn capabilities(&self) -> DataSourceCapabilities {
        DataSourceCapabilities::ALL
    }
}

/// The query-model schema of a batch's columns.
fn schema_of(batch: &RecordBatch) -> Result<Schema, DataSourceError> {
    let mut fields = Vec::with_capacity(batch.num_columns());
    for field in batch.schema().fields() {
        let data_type =
            DataType::from_arrow(field.data_type()).ok_or_else(|| DataSourceError::Backend {
                message: format!(
                    "column {} has the unknown type {}",
                    field.name(),
                    field.data_type()
                ),
            })?;
        let name = FieldName::new(field.name()).map_err(|error| DataSourceError::Backend {
            message: format!("column {}: {error}", field.name()),
        })?;
        fields.push(if field.is_nullable() {
            Field::new(name, data_type)
        } else {
            Field::required(name, data_type)
        });
    }
    Ok(Schema::new(fields))
}
