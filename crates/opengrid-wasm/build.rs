//! Lists the conformance cases for the browser test (`tests/conformance_in_browser.rs`).
//!
//! A browser test has no filesystem, so each case file has to be embedded with
//! `include_str!` — and a hand-kept list of them stopped at four of the 54. This
//! writes one test per file of `opengrid-conformance/cases/` into `OUT_DIR`, so a
//! new case runs in the browser without anyone adding it there.

use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let manifest =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("cargo sets the manifest dir"));
    let cases = manifest.join("../opengrid-conformance/cases");
    println!("cargo:rerun-if-changed={}", cases.display());

    let mut names: Vec<String> = fs::read_dir(&cases)
        .expect("the conformance cases are next to this crate")
        .map(|entry| entry.expect("a directory entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .filter_map(|path| {
            path.file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
        })
        .collect();
    names.sort();

    let mut out = String::new();
    for name in &names {
        let file = cases.join(format!("{name}.json"));
        out.push_str(&format!(
            "#[wasm_bindgen_test]\nfn case_{}() {{\n    check(include_str!({:?}));\n}}\n\n",
            name.replace('-', "_"),
            file.to_string_lossy(),
        ));
    }
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("cargo sets the out dir"));
    fs::write(out_dir.join("cases.rs"), out).expect("the list is written");
}
