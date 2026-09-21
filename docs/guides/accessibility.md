---
title: Accessibility
description: What the tests verify on every commit, and what has not been verified yet.
order: 3
---

Calling something accessible is easy and usually wrong, so here is the split.

## Built in

- `<opengrid-table>` and `<opengrid-pivot>` are native `<table>` elements. `<opengrid-grid>`
  uses `role="grid"`, because it takes over the arrow keys — for a report table that would be
  the wrong promise, which is why the table exists.
- Every feature is keyboard-operable. The grid follows the WAI-ARIA grid pattern; the key
  matrix is in [The public API](../../api/).
- Every state change goes through one polite live region.
- The component's own words are English and overridable with `set_texts`. A `lang` given with
  them goes onto the elements that carry **those texts**, never onto the data.
- A pivot group whose value is NULL is named `(no value)`, one whose value is the empty string
  `(empty)`: an empty header cell is silence to a screen reader.

## Verified by tests, on every commit

Roles, accessible names, `aria-rowcount` / `aria-rowindex` / `aria-sort` / `aria-selected`, the
roving tabindex and the keyboard matrix, target sizes, and the language of the component's
words versus the page's data. The Playwright suite in `tests/e2e/` runs axe-core over the
rendered elements.

`tests/e2e/announcements.spec.js` records the status line's **successive** states, so an
announcement that fires twice, never, or too early to survive the next result is a failing
test. The first time it ran, it found that turning a page announced nothing: the row count is
the same on every page, so the live region repeated itself while the content changed.

## Not yet verified

**A screen reader has not been through the finished V1.** The protocol exists and is the last
open item before a release. The e2e suite runs Chromium only.

So: the structure is tested hard, the experience is not signed off. Until it is, treat the
accessibility of this library as well-built and unaudited.
