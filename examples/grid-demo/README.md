# Grid-Demo (100k)

Manuelle Demo für `<opengrid-grid>` (Plan-Punkt 16): eine Seite, die einen
Datensatz über die lokale WASM-Engine lädt und vollständig per Tastatur bedienbar
ist. Die E2E-Suite bleibt klein; **diese Demo ist der Maßstabs-Check.**

## Starten

```console
just wasm-build            # Engine-Modul (einmalig, oder nach Engine-Änderungen)
just wasm-build-components # Element-Modul (nach Änderungen an den Komponenten)
just serve-demo            # Server-Wurzel ist das Repo
```

Dann <http://127.0.0.1:8080/examples/grid-demo/> öffnen.

## Optional: 100 000 Zeilen

Ohne Datensatz lädt die Demo den kleinen Conformance-Datensatz. Der 100k-Datensatz
liegt unter einem gitignorierten Pfad (`target/`) und wird nicht eingecheckt:

```console
mkdir -p target/grid-demo
cargo run -p xtask -- gen-orders --rows 100000 --seed 1 --out target/grid-demo/orders-100k.csv
```

## Tastatur

Pfeiltasten, `Home`/`End`, `Ctrl+Home`/`Ctrl+End`, `PageUp`/`PageDown`,
`Enter`/`Leertaste` auf einer Kopfzelle sortiert, `Escape` springt zur ersten
Zelle, `Tab`/`Shift+Tab` verlassen das Grid.

## Standard-Sortierung

Seiten brauchen eine totale Ordnung (Regel S6: `offset` ohne `sort` ist ein
Validierungsfehler). Das Grid sortiert deshalb beim Start nach seiner ersten
Spalte aufsteigend (`id`) und fällt darauf zurück, wenn die Sortierung
zurückgesetzt wird.
