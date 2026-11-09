//! The pool is the operator's to size (issue #220). The pool connects lazily,
//! so this needs no database.

use opengrid_datasource_postgres::{PoolOptions, PostgresDataSource};
use opengrid_types::{DataType, Field, FieldName, Schema};

fn schema() -> Schema {
    Schema::new(vec![Field::required(
        FieldName::new("id").unwrap(),
        DataType::Int64,
    )])
}

#[test]
fn the_pool_has_the_size_it_was_given() {
    let options = PoolOptions {
        max_size: Some(3),
        ..PoolOptions::default()
    };
    let source =
        PostgresDataSource::connect_with("host=localhost dbname=postgres", "t", schema(), options)
            .expect("a source");
    assert_eq!(source.pool_size(), 3);

    let default = PostgresDataSource::connect("host=localhost dbname=postgres", "t", schema())
        .expect("a source");
    assert_ne!(default.pool_size(), 3, "the default is the pool library's");
}
