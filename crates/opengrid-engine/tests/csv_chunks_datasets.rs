//! Issue #427: the conformance datasets — every type, derived columns, lists, trees, collation —
//! read piece by piece are the tables `load_csv` makes of them, whatever the piece size and
//! whatever size the loader was told the file has.

use opengrid_engine::ingest::{CsvLoader, CsvOptions, load_csv};
use opengrid_types::Schema;

const DATASETS: [&str; 6] = [
    "orders",
    "lists",
    "tree",
    "tree-cycle",
    "collation",
    "words",
];

fn read(name: &str, extension: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/../opengrid-conformance/data/{name}.{extension}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap_or_else(|error| panic!("{name}.{extension}: {error}"))
}

#[test]
fn every_dataset_in_pieces_is_the_dataset() {
    for name in DATASETS {
        let schema: Schema = opengrid_json::from_str(&read(name, "schema.json")).unwrap();
        let csv = read(name, "csv");
        let whole = load_csv(csv.as_bytes(), &schema, CsvOptions::default()).unwrap();
        assert!(whole.num_rows() > 0, "{name} has rows");
        for piece in [1, 7, 64, 1000, csv.len()] {
            // A size that is too small, right and too big: the columns grow, or are left roomy.
            for hint in [0, 10, csv.len(), csv.len() * 5] {
                let mut loader = CsvLoader::new(&schema, CsvOptions::default())
                    .unwrap()
                    .with_size_hint(hint);
                for chunk in csv.as_bytes().chunks(piece) {
                    loader.feed(chunk).unwrap();
                }
                // Compared as text: a NaN in the data is not equal to itself.
                let pieces = loader.finish().unwrap();
                assert_eq!(
                    format!("{pieces:?}"),
                    format!("{whole:?}"),
                    "{name}, pieces of {piece}, hint {hint}"
                );
            }
        }
    }
}
