# opengrid — Projektnotizen für Modelle

Portable Rust Data- und Query-Engine mit intelligenter Client-/Server-Ausführung;
darauf aufbauend ein kompromisslos barrierefreies DataGrid und PivotGrid über Web
Components. Das Produkt ist die Engine, nicht ein JS-Grid.

## Arbeitsanleitung

- Aufgaben stehen als **GitHub-Issues** (`casoon/opengrid`). Jede Issue ist in sich
  vollständig: was entschieden ist, die Aufgaben, „Done when" und „Not part of this".
  Was dort als entschieden steht, ist bindend.
- Pro Issue ein Branch und ein PR mit `Closes #N`. Nicht selbst nach `main` mergen.
- Lokal gibt es zusätzlich `plan/` (gitignored, nie committen) mit der ausführlichen
  Spezifikation. Wo es fehlt, gelten die Issue, `docs/` und die Kommentare im Code.

## Befehle

- `just check` — `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`
- `just wasm-check` — wasm32-Build aller wasm-fähigen Crates
- `just e2e` — baut die Module und fährt die Playwright-Suite (Chromium)
- `just fmt` — Quellcode formatieren

## Regeln

- **„Done when"-Befehle vor jedem Abschluss ausführen** und das Ergebnis im PR nennen;
  was sich in der Umgebung nicht ausführen ließ, ausdrücklich sagen (CI läuft auf PRs).
- Keine stillen Annahmen bei Grundsatzfragen (Query-Semantik, öffentliche API,
  neue Dependency, Sicherheit) — als Kommentar in der Issue notieren und dort stoppen.
- Tests müssen beißen: eine neue Prüfung per Mutation gegenprüfen.
- Nicht-Browser-Crates dürfen weder `web-sys` noch `js-sys` ziehen (Portabilität).
