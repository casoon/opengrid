# opengrid — Projektnotizen für Modelle

Portable Rust Data- und Query-Engine mit intelligenter Client-/Server-Ausführung;
darauf aufbauend ein kompromisslos barrierefreies DataGrid und PivotGrid über Web
Components. Das Produkt ist die Engine, nicht ein JS-Grid.

## Arbeitsanleitung

- Fahrplan und Arbeitsweise: **`plan/README.md`** und **`plan/status.md`**.
- Pro Session den nächsten offenen Punkt aus `plan/status.md` abarbeiten: Punktdatei
  lesen, dort unter „Kontext" genannte Spezifikations-Abschnitte lesen, umsetzen.
- Spezifikation ist bindend: `plan/spezifikation/`. `plan/` ist gitignored und wird
  nie committet.

## Befehle

- `just check` — `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`
- `just wasm-check` — wasm32-Build aller wasm-fähigen Crates
- `just fmt` — Quellcode formatieren

## Regeln

- **DoD-Befehle vor jedem Abschluss ausführen** und das Ergebnis nennen
  (plan/README.md → Definition of Done).
- Keine stillen Annahmen bei Grundsatzfragen (Query-Semantik, öffentliche API,
  neue Dependency, Sicherheit) — in der Punktdatei „Offene Fragen" notieren.
- Nicht-Browser-Crates dürfen weder `web-sys` noch `js-sys` ziehen
  (plan/spezifikation/11-crates.md §Portabilität).
