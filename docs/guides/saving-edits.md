---
title: Saving edits from the page
description: A grid that saves every cell on its own — naming the record, saying what became of an edit, and showing values the page computes.
order: 8
---

The grid edits; the page saves. opengrid has no write path — the engine's contract is a query —
so an edit leaves the grid as an `opengrid-cell-change` event, and storing it is the page's. This
guide is about a page that stores every cell as soon as the reader leaves it: a grade entry grid,
say, where each grade is saved on its own and an average next to it moves as the grades come in.

Such a page needs three things from the grid. The signatures are in
[The public API → Saving edits](../../api/#saving-edits); this page is about using them.

## Name the record

```html
<opengrid-grid label="Grades" datasource="grades" columns="student,name,test_1,test_2,average"
               row-key="student"></opengrid-grid>
```

`row-key` names the field that identifies a record. With it, `opengrid-cell-change` carries `key`,
the edited row's value of that field, next to `row`:

```js
grid.addEventListener("opengrid-cell-change", ({ detail }) => {
  detail.key;     // "s-1042" — the record
  detail.row;     // 7 — a position, right only until something sorts or reloads
  detail.column;  // "test_2"
  detail.value;   // "2.3", in the column's notation
});
```

Save by `key`. A save takes time, and in that time the reader may sort, filter or scroll — the
seventh row is then someone else. The key field should be one of the grid's `columns`; when the
result does not carry it, `key` is `null`.

## Say what became of the edit

```js
const { module } = await loadOpengrid();

grid.addEventListener("opengrid-cell-change", async ({ detail }) => {
  const { key, column, value } = detail;
  module.set_cell_state(grid, key, column, "saving");
  try {
    await fetch(`/api/grades/${key}/${column}`, { method: "PUT", body: value });
    module.set_cell_state(grid, key, column, "saved");
  } catch {
    module.set_cell_state(grid, key, column, "error", `Grade for ${key} was not saved.`);
  }
});
```

Until the page says otherwise, an edited value is marked unsaved. `saved` takes the mark off — no
reload needed. `error` keeps the value and the mark, since what the reader typed is still not
stored, and says the message in the grid's live region: the same single, polite region that
announces results, so a screen reader hears it without the focus moving. A failed cell also
carries `aria-invalid="true"`, so a reader who comes back to it later hears that something is
wrong with it.

Write the message for someone who is not looking at the cell: name the record and what failed.

**Styling.** The state is on the cell as `data-state`: `saving`, `saved` or `error`. The grid's
own sheet draws it quietly — the muted ink while saving, a wavy underline on a failure. The
attribute is inside the shadow root, and `::part(cell)` takes no attribute selector, so a page
cannot restyle the states from outside yet.

## Show values the page computes

```js
module.set_columns(grid, { average: { readonly: true, align: "end" } });

function showAverages(students) {
  module.set_values(
    grid,
    students.map((s) => ({ key: s.id, column: "average", value: s.average })),
  );
}
```

`set_values` puts values into cells without making them edits: no unsaved mark, no
`opengrid-cell-change`. Call it whenever the page has a new value — after a grade was saved, for
instance. A value stays with its record and stands over the source's value in every result that
follows, sorted or filtered, until the page sets another.

A column the page fills is `readonly`: it never opens an editor (an edit there would be
overwritten by the next value) and its cells say `aria-readonly="true"`. The column still has to
be one of the grid's `columns`; what the source has in it is shown until the page sets a value.

## What stays and what goes

- A state or a value names a **record**, so it outlives sorting, filtering and reloading.
- Another `row-key` or another `datasource` forgets them: the keys no longer mean the same records.
- A fresh result still replaces an edited value with the source's — the grid shows what the source
  says. The state the page set stays on the cell.
- Nothing here moves the focus.

Not part of this: undo, saving several cells as one batch, and conflicts between two readers
editing the same record. Those are the page's, together with the saving itself.
