# Remote-Demo (Browser → Server)

Dasselbe `<opengrid-grid>` wie die Grid-Demo, aber die Daten kommen über
`POST /query/orders` von einem laufenden `opengrid-server` (Plan-Punkte 23, 24,
27). Der Browser schickt den Query-AST, nie SQL.

## Starten

Zwei Prozesse, zwei Ports:

```console
just wasm-build-components                              # Element-Modul
cargo run -p opengrid-server -- examples/remote-demo/opengrid.toml   # :8081
just serve-demo                                         # :8080
```

Dann <http://127.0.0.1:8080/examples/remote-demo/> öffnen.

## Was die Seite zeigt

Drei Radiobuttons schalten das **Token** um — mehr ändert die Seite nicht. Der
Server hängt aus dem Token-Kontext einen **Pflichtfilter** an jede Abfrage
(E16), deshalb sehen die beiden Tokens verschiedene Zeilen:

| Token | Kontext | Ergebnis |
|---|---|---|
| `demo-token-de` | `country = "DE"` | 16 Zeilen, alle DE |
| `demo-token-fr` | `country = "FR"` | 10 Zeilen, alle FR |
| keins | — | `401`, Statuszeile: „The data could not be loaded: a valid bearer token is required" |

Der Fehlerfall ist der interessante: die Fehlerform aus Punkt 23 trägt den Satz
des Servers bis in die Statuszeile des Grids, und das Grid **bleibt stehen** —
Tabelle, Zeilen und Tastaturbedienung funktionieren weiter (Punkt 41).

## Was der Server nicht zulässt

- Eine Spalte außerhalb von `allowed_fields` (`note`, `flag`, `ratio`,
  `created_at`) ist für einen Client **nicht vorhanden**: die Antwort ist
  `422 unknown field "note"` — dieselbe Meldung wie bei einem Tippfehler.
- Den Pflichtfilter umgehen geht nicht: fragt das DE-Token nach `country = FR`,
  kommen null Zeilen, nicht die französischen.
- CORS ist per Default **aus**. `allowed_origins` in der Konfiguration listet die
  Origins einzeln; kein `*`, weil ein Wildcard zusammen mit einem Bearer-Token
  jedes offene Fenster berechtigen würde.

## Mit 100 000 Zeilen

`path` in `opengrid.toml` auf `../../target/grid-demo/orders-100k.csv` zeigen
lassen (Datensatz erzeugen: siehe `examples/grid-demo/README.md`) und
`allowed_fields`/`row_filter` an dessen Schema anpassen.
