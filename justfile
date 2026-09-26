# opengrid — Task-Runner (plan/spezifikation/14-entscheidungen.md E11)

set shell := ["bash", "-uc"]

# Panic locations are baked into a release binary as **absolute** paths, so an
# unremapped `.wasm` carries the build machine's home directory — the
# developer's username handed to everyone who installs the package, and useless
# to them. Measured before this existed: 27 such paths in the shipped module,
# 16 from the toolchain and 11 from the registry.
#
# `--remap-path-prefix` is the stable way; Cargo's `profile.trim-paths` is not
# stabilized in the pinned toolchain (checked 2026-09-20, Cargo 1.98). The
# prefixes are read from the environment, so this works on any machine.
# tests/e2e/packaged.spec.js fails if a path ever leaks again.
export REMAP := "--remap-path-prefix=" + env_var("HOME") + "/.cargo=/cargo --remap-path-prefix=" + env_var("HOME") + "/.rustup=/rustup --remap-path-prefix=" + justfile_directory() + "=/opengrid"

# Alles, was jede Session prüfen muss (plan/spezifikation/12-qualitaet.md §CI).
check:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    cargo test --workspace --all-features

# Build aller wasm-fähigen Crates für wasm32-unknown-unknown.
wasm-check:
    cargo build --target wasm32-unknown-unknown -p opengrid-types -p opengrid-query -p opengrid-arrow-engine -p opengrid-datasource -p opengrid-wasm -p opengrid-web-core -p opengrid-web-components -p opengrid-grid -p opengrid-export

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
    RUSTFLAGS="$REMAP" cargo build --release --target wasm32-unknown-unknown -p opengrid-wasm
    wasm-bindgen --target web --out-dir examples/engine-demo/pkg --out-name opengrid_wasm "${CARGO_TARGET_DIR:-target}/wasm32-unknown-unknown/release/opengrid_wasm.wasm"
    wasm-opt -Oz -o examples/engine-demo/pkg/opengrid_wasm_bg.wasm examples/engine-demo/pkg/opengrid_wasm_bg.wasm

# Demo lokal ausliefern. Server-Wurzel ist das Repo, weil die Demo den
# Conformance-Datensatz lädt (crates/opengrid-conformance/data/).
serve-demo:
    python3 -m http.server 8080

# Browser-Modul der Custom Elements; loader.js erwartet es unter packages/opengrid/pkg/.
# Getrennt vom Engine-Demo-Modul (die Modultrennung `grid.wasm` macht Punkt 40).
wasm-build-components:
    RUSTFLAGS="$REMAP" cargo build --release --target wasm32-unknown-unknown -p opengrid-web-components
    wasm-bindgen --target web --out-dir packages/opengrid/pkg --out-name opengrid_web_components "${CARGO_TARGET_DIR:-target}/wasm32-unknown-unknown/release/opengrid_web_components.wasm"
    wasm-opt -Oz -o packages/opengrid/pkg/opengrid_web_components_bg.wasm packages/opengrid/pkg/opengrid_web_components_bg.wasm

# Packt `@casoon/opengrid` wie ein Release und entpackt es nach
# target/npm-package/package (Punkt 40, E25). Baut das Element-Modul mit.
# tests/e2e/packaged.spec.js lädt genau daraus — nicht aus dem Repository.
package:
    bash scripts/pack-npm.sh

# Typen der öffentlichen API (Punkt 75): tests/types/api.ts gegen das Repository
# und gegen das gepackte Paket — dort über dessen `exports`, wie bei einem Nutzer.
# Dazu die Framework-Adapter (Punkte 77–79): ihre Typen im Beispiel, und die
# gepackten Pakete in einem Wegwerf-Projekt, serverseitig gerendert und typgeprüft.
# Braucht `just package` vorher; `just e2e` ruft es auf.
types:
    pnpm exec tsc -p tests/types
    bash scripts/typecheck-package.sh
    pnpm exec tsc -p examples/react
    pnpm exec tsc -p examples/vue
    pnpm exec tsc -p examples/svelte
    bash scripts/check-adapter-package.sh react
    bash scripts/check-adapter-package.sh vue
    bash scripts/check-adapter-package.sh svelte

# Größen der Elementmodule: beide Elemente, nur Grid, nur Pivot — roh, gzip,
# brotli (Punkt 40). Die Zahlen stehen in plan/spezifikation/12-qualitaet.md.
measure-modules:
    bash scripts/measure-modules.sh

# End-to-End- und A11y-Tests (Playwright + axe-core, plan/spezifikation/12-qualitaet.md §CI).
# Baut das Element-Modul und das Engine-Modul (die Fixture fährt die echte Engine),
# packt das npm-Paket (packaged.spec.js prüft das gepackte, nicht das Repository),
# installiert die gepinnte JS-Toolchain und fährt tests/e2e/.
e2e: wasm-build-components wasm-build package
    # Die Hybrid-Specs (Punkt 28) fahren gegen einen echten opengrid-server, den
    # Playwright startet. Hier gebaut, damit dort nur noch gestartet wird — ein
    # Kaltbau innerhalb des webServer-Timeouts wäre ein Glücksspiel.
    cargo build -p opengrid-server
    pnpm install --frozen-lockfile
    pnpm --filter "./examples/*" build
    just types
    pnpm exec playwright test --config tests/e2e/playwright.config.js

# Dieselbe Suite in Firefox und WebKit (Release-Checkliste, docs/releasing.md).
# Opt-in: `just e2e` und CI bleiben bei Chromium. Ohne Screenshot-Baselines —
# die gehören Chromium. Die Browser einmal holen:
# `pnpm exec playwright install firefox webkit`.
e2e-browsers: wasm-build-components wasm-build package
    cargo build -p opengrid-server
    pnpm install --frozen-lockfile
    pnpm --filter "./examples/*" build
    OPENGRID_E2E_BROWSERS=1 pnpm exec playwright test --config tests/e2e/playwright.config.js --project firefox --project webkit

# Setzt die Release-Version an allen drei Stellen, die eine drucken: Cargo-
# Workspace, npm-Paket und der Kopf der Projektseite (Punkt 40). Sie sind
# einmal auseinandergelaufen, und die Seite zeigt ihre Version öffentlich.
# Beispiel: `just set-version 0.1.0`
set-version version:
    bash scripts/set-version.sh {{version}}

# Native criterion-Benchmarks der Engine (plan/spezifikation/12-qualitaet.md §Benchmarks).
bench-native:
    cargo bench -p opengrid-arrow-engine

# Dieselben Operationen im WASM-Build, headless Chrome wie `wasm-test` (E4).
bench-wasm:
    CHROMEDRIVER=chromedriver cargo bench --target wasm32-unknown-unknown -p opengrid-wasm --bench engine

# Beide Benchmark-Suiten nacheinander.
bench: bench-native bench-wasm
