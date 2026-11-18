//! What the tests share: a SQLite file with a table to propose and serve.

#![allow(dead_code)]

use std::path::PathBuf;

use opengrid_connector_sqlite::Connection;

/// A database file of its own per test, removed with the guard.
pub struct Database {
    pub path: PathBuf,
}

impl Database {
    pub fn new(name: &str, sql: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "opengrid-serve-{}-{name}.sqlite",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        Connection::open(&path)
            .expect("a database file")
            .execute_batch(sql)
            .expect("the tables");
        Self { path }
    }

    /// The connection string for the environment: `sqlite:<path>`.
    pub fn connection(&self) -> String {
        format!("sqlite:{}", self.path.display())
    }
}

impl Drop for Database {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// A table with every supported type, and four columns that have none.
pub const SQLITE_TABLES: &str = "
    CREATE TABLE orders (
        id INTEGER PRIMARY KEY,
        customer TEXT,
        country VARCHAR(2) NOT NULL,
        qty BIGINT,
        ratio REAL,
        paid BOOLEAN,
        amount DECIMAL(12,2),
        ordered_on DATE,
        payload BLOB,
        secret_note TEXT
    );
    INSERT INTO orders VALUES
        (1, 'Ada', 'DE', 3, 0.5, 1, 10.5, '2026-01-02', NULL, 'do not serve'),
        (2, 'Bo', 'FR', 7, 1.5, 0, 20.0, '2026-02-03', NULL, 'do not serve');
";
