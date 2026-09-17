# Grid-Demo (100k)

Manuelle Demo für `<opengrid-grid>` (Plan-Punkte 16/17): eine Seite, die einen
Datensatz über die lokale WASM-Engine lädt, vollständig per Tastatur bedienbar ist
und bei 100 000 logischen Zeilen nur ein rund 40 Zeilen großes DOM-Fenster hält
(Row-Recycling). Die E2E-Suite bleibt klein; **diese Demo ist der Maßstabs-Check.**

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
`Enter`/`Leertaste` auf einer Kopfzelle sortiert, `Shift`+`Enter`/`Leertaste`
ergänzt bzw. entfernt die Spalte als weiteren Sortierschlüssel (die Kopfzeile
zeigt die Reihenfolge als Index), `Escape` springt zur ersten Zelle,
`Tab`/`Shift+Tab` verlassen das Grid. `Ctrl+End` lädt die letzte logische
Zeile (`aria-rowindex=100001`), rendert sie und fokussiert sie.

## Filter

Über der Tabelle sitzt eine typ-agnostische Filterzeile (`part="filter"`): je
Spalte ein Operator-`<select>` (`contains`, `starts_with`, `eq`, `ne`, `gt`,
`gte`, `lt`, `lte`) und ein Wert-`<input>`, beide mit `aria-label`. `Enter` im
Eingabefeld wendet den `and`-Filter an, „Clear" leert ihn, die Trefferzahl
steht als `role="status"` daneben. Phase B kennt noch keine Spaltentypen
(Punkt 23), die Werte gehen deshalb als Strings in die Query — Filter auf
Textspalten wie `customer`/`country` funktionieren, numerische Filter folgen
mit Punkt 23.

## Virtualisierung (Punkt 17)

Das `<tbody>` ist der Sizer (`height = total_count * 32px`), die Pool-Zeilen sind
`position: absolute` und werden per `translateY(zelle * 32px)` platziert; `<thead>`
ist `position: sticky`. Beim Scrollen wird nur das Fenster (`limit=40`,
`offset=Fensterstart`) nachgeladen und die vorhandenen Zeilen werden recycelt —
es entstehen keine neuen Knoten. Die Zeile mit dem fokussierten Feld wird nie
recycelt, damit der Fokus das Scrollen überlebt.

### Messwert (Chrome headless, 100 000 Zeilen, 5 Spalten)

Ein Scroll über die gesamte Höhe in 300 Schritten (`requestAnimationFrame`,
jeder Schritt setzt `scrollTop`):

| Kennzahl | Wert |
|---|---|
| DOM-Zeilen vorher / nachher | 40 / 40 (= Pool, konstant) |
| DOM-Zellen vorher / nachher | 200 / 200 |
| `aria-rowcount` | 100001 |
| Schrittzahl | 300 |
| Dauer | 5000 ms (≈ 16,7 ms/Schritt) |
| FPS | 60 |
| `Ctrl+End` | fokussiert `data-row=99999`, `aria-rowindex=100001` |

## Standard-Sortierung

Seiten brauchen eine totale Ordnung (Regel S6: `offset` ohne `sort` ist ein
Validierungsfehler). Das Grid sortiert deshalb beim Start nach seiner ersten
Spalte aufsteigend (`id`) und fällt darauf zurück, wenn die Sortierung
zurückgesetzt wird.
