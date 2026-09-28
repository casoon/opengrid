//! `opengrid-conformance --endpoint <url> --token <token>` (issue #48): the
//! whole suite over HTTP against any server, in both result forms.
//!
//! The server must hold the fixture (`crates/opengrid-conformance/data/`) as a
//! source named `orders`, every column allowed, no row filter. Exit code 0
//! when every case agrees, 1 when one does not, 2 for a usage error.

fn main() {
    let mut endpoint = None;
    let mut token = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--endpoint" => endpoint = args.next(),
            "--token" => token = args.next(),
            _ => usage(),
        }
    }
    let (Some(endpoint), Some(token)) = (endpoint, token) else {
        usage();
    };
    match opengrid_conformance::check_endpoint(&endpoint, &token) {
        Ok(report) => {
            println!("{endpoint}: {report}");
            std::process::exit(if report.is_ok() { 0 } else { 1 });
        }
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    }
}

fn usage() -> ! {
    eprintln!("usage: opengrid-conformance --endpoint http://host:port --token <token>");
    std::process::exit(2);
}
