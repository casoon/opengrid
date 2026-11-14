//! Issue #427: a CSV read in pieces is the CSV read at once — the same table, or the same error
//! with the same line — wherever the pieces are cut: inside a field, inside quotes, inside a
//! character, between the CR and the LF.

use opengrid_engine::Table;
use opengrid_engine::ingest::{CsvLoader, CsvOptions, IngestError, load_csv};
use opengrid_types::{DataType, Field, FieldName, Schema};
use proptest::prelude::*;

fn schema() -> Schema {
    Schema::new(vec![
        Field::required(FieldName::new("id").unwrap(), DataType::Int64),
        Field::new(FieldName::new("name").unwrap(), DataType::Utf8),
        Field::new(FieldName::new("amount").unwrap(), DataType::Float64),
    ])
}

fn at_once(bytes: &[u8]) -> Result<Table, IngestError> {
    load_csv(bytes, &schema(), CsvOptions::default())
}

fn in_pieces(bytes: &[u8], cuts: &[usize], hint: usize) -> Result<Table, IngestError> {
    let mut loader = CsvLoader::new(&schema(), CsvOptions::default())?.with_size_hint(hint);
    let mut from = 0;
    for &cut in cuts {
        let cut = cut.min(bytes.len()).max(from);
        loader.feed(&bytes[from..cut])?;
        from = cut;
    }
    loader.feed(&bytes[from..])?;
    loader.finish()
}

fn cell() -> impl Strategy<Value = String> {
    prop_oneof![
        "[a-z0-9 ä€😀]{0,6}".prop_map(|s| s),
        "[a-z ,\n\r]{0,6}".prop_map(|s| format!("\"{}\"", s)),
        Just("\"say \"\"hi\"\"\"".to_owned()),
        Just("\"a\"\"\n,b\"".to_owned()),
        Just("a\"b".to_owned()),
        Just("\\N".to_owned()),
        Just("1.5".to_owned()),
        Just("x".to_owned()),
        Just("-3".to_owned()),
        Just("".to_owned()),
    ]
}

fn file() -> impl Strategy<Value = Vec<u8>> {
    let record = prop::collection::vec(cell(), 1..5).prop_map(|cells| cells.join(","));
    (
        prop::collection::vec(record, 0..12),
        prop_oneof![Just("\n"), Just("\r\n"), Just("\r")],
        any::<bool>(),
        any::<bool>(),
    )
        .prop_map(|(records, eol, header, bom)| {
            let mut text = String::new();
            if bom {
                text.push('\u{feff}');
            }
            if header {
                text.push_str("id,name,amount");
                text.push_str(eol);
            }
            text.push_str(&records.join(eol));
            if !records.is_empty() && eol != "\r" {
                text.push_str(eol);
            }
            text.into_bytes()
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1500))]

    #[test]
    fn pieces_are_the_whole(
        bytes in file(),
        cuts in prop::collection::vec(0usize..200, 0..6),
        hint in prop_oneof![Just(0usize), Just(1usize), Just(5000usize)],
    ) {
        let mut cuts = cuts;
        cuts.sort_unstable();
        let whole = at_once(&bytes);
        let pieces = in_pieces(&bytes, &cuts, hint);
        match (&whole, &pieces) {
            (Ok(a), Ok(b)) => prop_assert_eq!(a, b),
            (Err(a), Err(b)) => prop_assert_eq!(a, b),
            _ => prop_assert!(false, "one failed: {whole:?} / {pieces:?} for {:?}", String::from_utf8_lossy(&bytes)),
        }
    }
}

#[test]
fn a_file_cut_into_single_bytes_is_the_file() {
    let bytes = "id,name,amount\n1,\"a\nb\",1.5\r\n2,ä€,\\N\n3,\"say \"\"hi\"\"\",2".as_bytes();
    let cuts: Vec<usize> = (1..bytes.len()).collect();
    assert_eq!(at_once(bytes).unwrap(), in_pieces(bytes, &cuts, 0).unwrap());
}

#[test]
fn an_error_names_the_line_it_is_on_across_pieces() {
    let bytes = b"id,name,amount\n1,a,1\n2,b\n3,c,3\n";
    let error = in_pieces(bytes, &[16, 19], 0).unwrap_err();
    assert_eq!(error, at_once(bytes).unwrap_err());
    assert!(error.to_string().contains("line 3"), "{error}");
}
