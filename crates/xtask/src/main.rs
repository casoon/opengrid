//! `xtask` — repo tooling from the command line (plan point 11).
//!
//! ```console
//! cargo run -p xtask -- gen-orders --rows 100000 --seed 1 --out orders_100k.csv
//! ```
//!
//! Without `--out` the CSV goes to stdout. The dataset is deterministic in
//! `(rows, seed)`; see [`xtask::orders_csv`].

use std::io::Write;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("xtask: {message}");
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

const USAGE: &str = "\
usage: xtask gen-orders [--rows N] [--seed S] [--out FILE]

  --rows N    number of data rows (default 100000)
  --seed S    deterministic seed (default 1)
  --out FILE  write here instead of stdout";

fn run(args: Vec<String>) -> Result<(), String> {
    let Some(command) = args.first() else {
        return Err("no command".to_owned());
    };
    if command != "gen-orders" {
        return Err(format!("unknown command {command:?}"));
    }

    let mut rows = 100_000usize;
    let mut seed = 1u64;
    let mut out: Option<String> = None;

    let mut index = 1;
    while index < args.len() {
        let value = || {
            args.get(index + 1)
                .cloned()
                .ok_or_else(|| format!("{} needs a value", args[index]))
        };
        match args[index].as_str() {
            "--rows" => {
                rows = value()?
                    .parse()
                    .map_err(|error| format!("--rows: {error}"))?;
                index += 2;
            }
            "--seed" => {
                seed = value()?
                    .parse()
                    .map_err(|error| format!("--seed: {error}"))?;
                index += 2;
            }
            "--out" => {
                out = Some(value()?);
                index += 2;
            }
            other => return Err(format!("unknown argument {other:?}")),
        }
    }

    let csv = xtask::orders_csv(rows, seed);
    match out {
        Some(path) => std::fs::write(&path, csv).map_err(|error| format!("{path}: {error}"))?,
        None => std::io::stdout()
            .write_all(csv.as_bytes())
            .map_err(|error| error.to_string())?,
    }
    Ok(())
}
