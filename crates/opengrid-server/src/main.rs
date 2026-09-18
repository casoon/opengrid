//! The binary: `opengrid-server <config.toml>`.

use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let Some(config_path) = args.next().map(PathBuf::from) else {
        eprintln!("usage: opengrid-server <config.toml>");
        std::process::exit(2);
    };

    let (state, config) = opengrid_server::build(&config_path)?;
    let sources = state.registry.names().join(", ");
    let tokens = state.tokens.len();
    let listener = tokio::net::TcpListener::bind(&config.server.address).await?;

    // What is on and what is off, said once, at startup — a gateway whose
    // security depends on configuration should not keep that to itself.
    println!(
        "opengrid-server on http://{} — sources: {sources} — {tokens} token(s) configured",
        listener.local_addr()?
    );
    if tokens == 0 {
        println!("  no tokens configured: every request will be refused");
    }

    axum::serve(listener, opengrid_server::router(state)).await?;
    Ok(())
}
