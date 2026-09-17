# opengrid — Task-Runner (plan/spezifikation/14-entscheidungen.md E11)

set shell := ["bash", "-uc"]

# Alles, was jede Session prüfen muss (plan/spezifikation/12-qualitaet.md §CI).
check:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    cargo test --workspace --all-features

# Build aller wasm-fähigen Crates für wasm32-unknown-unknown.
wasm-check:
    cargo build --target wasm32-unknown-unknown -p opengrid-types -p opengrid-query -p opengrid-arrow-engine -p opengrid-datasource -p opengrid-wasm -p opengrid-web-core -p opengrid-web-components -p opengrid-grid

# Quellcode formatieren.
fmt:
    cargo fmt --all

# Conformance-Suite und Demo-Query im echten Browser (E4): `wasm-bindgen-test-runner`
# treibt headless Chrome über `chromedriver`. Chrome ist Pflicht, also wird der
# Treiber explizit gewählt — sonst nimmt der Runner den vorinstallierten Safari
# (Risiko R7).
wasm-test:
    CHROMEDRIVER=chromedriver cargo test --target wasm32-unknown-unknown -p opengrid-wasm --test conformance_in_browser
    CHROMEDRIVER=chromedriver cargo test --target wasm32-unknown-unknown -p opengrid-web-components --test element

# WASM-Modul für die Demo bauen (plan/spezifikation/14-entscheidungen.md E4):
# cargo release -> wasm-bindgen --target web -> wasm-opt -Oz.
# Die Crate pinnt `wasm-bindgen` auf dieselbe Version wie diese CLI; eine
# abweichende CLI bricht mit einem Versionsfehler ab (Risiko R7).
# `${CARGO_TARGET_DIR:-target}` respektiert ein gesetztes Zielverzeichnis.
wasm-build:
    cargo build --release --target wasm32-unknown-unknown -p opengrid-wasm
    wasm-bindgen --target web --out-dir examples/engine-demo/pkg --out-name opengrid_wasm "${CARGO_TARGET_DIR:-target}/wasm32-unknown-unknown/release/opengrid_wasm.wasm"
    wasm-opt -Oz -o examples/engine-demo/pkg/opengrid_wasm_bg.wasm examples/engine-demo/pkg/opengrid_wasm_bg.wasm

# Demo lokal ausliefern. Server-Wurzel ist das Repo, weil die Demo den
# Conformance-Datensatz lädt (crates/opengrid-conformance/data/).
serve-demo:
    python3 -m http.server 8080

# Browser-Modul der Custom Elements; loader.js erwartet es unter packages/opengrid/pkg/.
# Getrennt vom Engine-Demo-Modul (die Modultrennung `grid.wasm` macht Punkt 40).
wasm-build-components:
    cargo build --release --target wasm32-unknown-unknown -p opengrid-web-components
    wasm-bindgen --target web --out-dir packages/opengrid/pkg --out-name opengrid_web_components "${CARGO_TARGET_DIR:-target}/wasm32-unknown-unknown/release/opengrid_web_components.wasm"
    wasm-opt -Oz -o packages/opengrid/pkg/opengrid_web_components_bg.wasm packages/opengrid/pkg/opengrid_web_components_bg.wasm

# End-to-End- und A11y-Tests (Playwright + axe-core, plan/spezifikation/12-qualitaet.md §CI).
# Baut das Element-Modul und das Engine-Modul (die Fixture fährt die echte Engine),
# installiert die gepinnte JS-Toolchain und fährt tests/e2e/.
e2e: wasm-build-components wasm-build
    pnpm install --frozen-lockfile
    pnpm exec playwright test --config tests/e2e/playwright.config.js

# Native criterion-Benchmarks der Engine (plan/spezifikation/12-qualitaet.md §Benchmarks).
bench-native:
    cargo bench -p opengrid-arrow-engine

# Dieselben Operationen im WASM-Build, headless Chrome wie `wasm-test` (E4).
bench-wasm:
    CHROMEDRIVER=chromedriver cargo bench --target wasm32-unknown-unknown -p opengrid-wasm --bench engine

# Beide Benchmark-Suiten nacheinander.
bench: bench-native bench-wasm
