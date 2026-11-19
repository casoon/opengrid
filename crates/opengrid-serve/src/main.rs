//! `opengrid-server propose` and `opengrid-server --config <file>`.

use std::process::ExitCode;

use opengrid_serve::{Config, DEFAULT_CONNECTION_ENV, read_proposal};

const USAGE: &str = "usage:
  opengrid-server propose [--database-env VAR] [--namespace SCHEMA] [--out FILE]
      read the schema behind the connection string in $VAR (default OPENGRID_DATABASE_URL;
      postgres://…, mysql://…/<database> or sqlite:<path>) and write a configuration proposal (default opengrid.json)
  opengrid-server --config FILE
      serve what FILE names; tokens and connection strings come from the environment";

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let outcome = match args.first().map(String::as_str) {
        Some("propose") => propose(&args[1..]).await,
        Some("--config") if args.len() == 2 => serve(&args[1]).await,
        _ => Err(USAGE.to_owned()),
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::from(2)
        }
    }
}

async fn propose(args: &[String]) -> Result<(), String> {
    let mut env = DEFAULT_CONNECTION_ENV.to_owned();
    let mut namespace = "public".to_owned();
    let mut out = "opengrid.json".to_owned();
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        let value = args.next().ok_or_else(|| USAGE.to_owned())?;
        match flag.as_str() {
            "--database-env" => env = value.clone(),
            "--namespace" => namespace = value.clone(),
            "--out" => out = value.clone(),
            _ => return Err(USAGE.to_owned()),
        }
    }
    let connection =
        std::env::var(&env).map_err(|_| format!("the environment variable {env} is not set"))?;
    // The file is the owner's to edit: never overwrite one.
    if std::path::Path::new(&out).exists() {
        return Err(format!("{out} exists; remove it or choose another --out"));
    }
    let (text, skipped) = read_proposal(&connection, &env, &namespace)
        .await
        .map_err(|e| e.to_string())?;
    std::fs::write(&out, text).map_err(|e| format!("{out}: {e}"))?;
    for name in skipped {
        eprintln!("skipped the table {name:?}: its name is not a valid identifier");
    }
    eprintln!("wrote {out}: read it, edit it, and delete \"proposal\": true to serve it");
    Ok(())
}

async fn serve(path: &str) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let config = Config::parse(&text).map_err(|e| format!("{path}: {e}"))?;
    let server = config
        .into_builder(|name| std::env::var(name).ok())
        .map_err(|e| e.to_string())?
        .build()
        .await
        .map_err(|e| e.to_string())?;
    let listener = tokio::net::TcpListener::bind(&config.listen)
        .await
        .map_err(|e| format!("{}: {e}", config.listen))?;
    println!(
        "opengrid-server on http://{} — sources: {}",
        listener.local_addr().map_err(|e| e.to_string())?,
        server.state().registry().names().join(", ")
    );
    axum::serve(listener, server.router())
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .map_err(|e| e.to_string())
}
