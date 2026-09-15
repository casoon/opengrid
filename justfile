# opengrid — Task-Runner (plan/spezifikation/14-entscheidungen.md E11)

set shell := ["bash", "-uc"]

# Alles, was jede Session prüfen muss (plan/spezifikation/12-qualitaet.md §CI).
check:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    cargo test --workspace --all-features

# Build aller wasm-fähigen Crates für wasm32-unknown-unknown.
wasm-check:
    cargo build --target wasm32-unknown-unknown -p opengrid-types

# Quellcode formatieren.
fmt:
    cargo fmt --all
