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
ergänzt bzw. entfernt die Spalte als weiteren Sortierschlüssel, `Escape` springt
zur ersten Zelle, `Tab`/`Shift+Tab` verlassen das Grid. `Ctrl+End` lädt die
letzte logische Zeile (`aria-rowindex=100001`), rendert sie und fokussiert sie.

Die Kopfzelle zeigt die **Richtung** als ▲/▼ und bei Mehrfachsortierung
zusätzlich die Position (1, 2). Beide Marken sind `aria-hidden` — für assistive
Technik steht die Richtung in `aria-sort`, sie wird also nicht doppelt angesagt
(Punkt 49).

Eine fokussierte Zelle, deren Wert breiter ist als die Spalte, **entfaltet sich**
und zeigt ihn vollständig über den Zeilen darunter; beim Weitergehen klappt sie
zurück (Punkt 47). Im DOM stand der volle Wert ohnehin immer — gekürzt hat nur
die Anzeige.

## Filter und Statuszeile

Über der Tabelle sitzt eine typ-agnostische Filterzeile (`part="filter"`): je
Spalte ein Operator-`<select>` und ein Wert-`<input>`, beide mit `aria-label`.
`Enter` im Eingabefeld wendet den `and`-Filter an, „Clear" leert ihn. Die
Auswahl zeigt lesbare Bezeichnungen („greater or equal"); der `value` bleibt
das Wire-Token (`gte`), die Query ändert sich dadurch nicht. Phase B kennt noch
keine Spaltentypen (Punkt 23), die Werte gehen deshalb als Strings in die Query
— Filter auf Textspalten wie `customer`/`country` funktionieren, numerische
folgen mit Punkt 23.

Darunter liegt die **Statuszeile** (`part="status"`, `role="status"`,
`aria-live="polite"`) — die einzige Live-Region des Grids. Sie trägt alle vier
Zustände: „Loading …", „N matches", „No matches" und, bei einer fehlgeschlagenen
Abfrage, „The data could not be loaded: …". Ein Fehler ersetzt das Grid **nicht**:
Tabelle, Zeilen und Tastaturbedienung bleiben, nur die Zeile ändert sich
(Punkt 41). Scrollen sagt bewusst kein „Laden" an, sonst plapperte die
Live-Region einmal pro Frame.

## Sprache der Texte

Die eingebauten Texte der Komponente sind **englisch** und mit `lang="en"`
ausgezeichnet — auf dieser deutschen Seite wechselt ein Screenreader dort also
die Stimme, und das ist richtig so. Eine Seite setzt eigene Texte mit
`set_texts(host, …)`; diese Demo legt dafür `window.opengrid` in der Konsole ab
(Punkt 48, Snippet im Testprotokoll `plan/21-sr-protokoll.md`).

## Theming

Custom Properties auf dem Host (`--grid-row-height`, `--grid-header-height`,
`--grid-border-color`, `--grid-focus-width`, `--grid-filter-height`,
`--grid-status-height`) und `::part(…)` für Aufbau und Marken. Fokusring,
Systemfarben unter `forced-colors` und `prefers-reduced-motion` kann ein Theme
nicht abschalten (Punkt 20).

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
