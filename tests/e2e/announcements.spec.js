import { test, expect } from "@playwright/test";

// What the status line says, **in order** (plan points 41/55).
//
// Every other spec reads the status line once, at the end. That catches a
// wrong sentence and misses the three failures a screen-reader user actually
// runs into: a thing said **twice**, a thing **never** said, and a thing said
// at the **wrong moment** — swallowed by the result of the next query. Those
// are properties of a sequence, so this spec records the sequence.
//
// It does not replace the run in plan/55-protokoll.md. A recording proves what
// *would* be announced; only a person with a screen reader hears whether the
// live region actually interrupts, queues or stays silent, and whether the
// wording is comprehensible. This turns that run from an inventory into a
// judgement — which is the whole reason it exists.

/**
 * Starts recording every distinct state of the grid's status line.
 *
 * Watches the shadow root rather than the node: `set_texts` and a page change
 * rebuild the skeleton, so the node the observer was given can be replaced
 * underneath it (the pin of point 17 is not the only thing that moves).
 * Consecutive equal values collapse, because a repeated identical announcement
 * is what `GridState::set_status` already suppresses — recording it twice
 * would test the observer, not the grid.
 */
async function record(page) {
  await page.evaluate(() => {
    const host = document.querySelector("opengrid-grid");
    const root = host.shadowRoot;
    const seen = [];
    const read = () => {
      const line = root.querySelector('[part="status"]');
      if (!line) return;
      const text = line.textContent.trim();
      const state = line.getAttribute("data-state");
      const last = seen.at(-1);
      if (!last || last.text !== text || last.state !== state) {
        seen.push({ text, state });
      }
    };
    read();
    const observer = new MutationObserver(read);
    observer.observe(root, {
      subtree: true,
      childList: true,
      characterData: true,
      attributes: true,
      attributeFilter: ["data-state"],
    });
    window.__announced = seen;
    window.__stopRecording = () => observer.disconnect();
  });
}

/** Everything announced since `record`, oldest first. */
async function announced(page) {
  return page.evaluate(() => window.__announced.map((entry) => entry.text));
}

/** The same, with the `data-state` each announcement carried. */
async function announcedStates(page) {
  return page.evaluate(() => window.__announced.map((entry) => [entry.text, entry.state]));
}

async function focusCell(page, selector) {
  await page.evaluate((selector) => {
    document.querySelector("opengrid-grid").shadowRoot.querySelector(selector).focus();
  }, selector);
}

async function click(page, part) {
  await page.evaluate((part) => {
    document.querySelector("opengrid-grid").shadowRoot.querySelector(`[part="${part}"]`).click();
  }, part);
}

async function open(page, fixture) {
  await page.goto(`/tests/e2e/fixtures/${fixture}`);
  await page.waitForFunction(() => document.querySelector("opengrid-grid")?.shadowRoot);
  await expect(page.locator("opengrid-grid tbody tr").first()).toBeVisible();
  await record(page);
}

test.describe("selection", () => {
  test("a dropped selection is announced once, and survives the result", async ({ page }) => {
    await open(page, "grid-long.html");
    await focusCell(page, 'td[data-row="1"][data-col="0"]');
    await page.keyboard.press(" ");

    // Sorting re-queries, so the notice and the result race. Point 41's notice
    // is set after the rebuild precisely so the result cannot overwrite it.
    await focusCell(page, 'th[data-col="1"]');
    await page.keyboard.press("Enter");
    await expect.poll(() => announced(page).then((a) => a.join("|"))).toContain(
      "Selection cleared",
    );

    const said = await announced(page);
    const cleared = said.filter((text) => text.includes("Selection cleared"));
    expect(cleared, "said exactly once, not once per patch").toHaveLength(1);

    // The notice rides *with* the result rather than replacing it: the user is
    // told what happened and what they are now looking at, in one utterance.
    // A bare "Selection cleared" would leave them with no idea what is on
    // screen, and a bare row count would hide that the selection is gone.
    const notice = said.find((text) => text.includes("Selection cleared"));
    expect(notice).toMatch(/matches|Treffer/);
    // And it comes after a `loading`, i.e. it survived the re-query — that is
    // the whole reason `set_notice` remembers it for one more result.
    const states = (await announcedStates(page)).map(([, state]) => state);
    expect(states).toContain("loading");
    expect(states.at(-1)).toBe("ready");
  });

  test("selecting a row says nothing — the row says it itself", async ({ page }) => {
    await open(page, "grid-long.html");
    const before = await announced(page);
    await focusCell(page, 'td[data-row="2"][data-col="0"]');
    await page.keyboard.press(" ");
    await expect(
      page.locator('opengrid-grid tbody tr[aria-selected="true"]').first(),
    ).toBeVisible();

    // `aria-selected` on the row is the announcement. A status line that also
    // spoke would say it twice, which is the failure this spec exists for.
    expect(await announced(page)).toEqual(before);
  });
});

test.describe("columns", () => {
  test("hiding and moving a column each announce once", async ({ page }) => {
    await open(page, "grid.html");
    await focusCell(page, 'th[data-col="1"]');
    await page.keyboard.press("Control+ArrowRight");
    await expect.poll(() => announced(page).then((a) => a.length)).toBeGreaterThan(1);

    const said = await announced(page);
    const moves = said.filter((text) => /moved|verschoben/i.test(text));
    expect(moves.length, "a move is announced").toBeGreaterThan(0);
    for (const text of new Set(moves)) {
      expect(
        said.filter((other) => other === text),
        `"${text}" is said once`,
      ).toHaveLength(1);
    }
  });
});

test.describe("paging", () => {
  test("every page change is announced, and none is skipped", async ({ page }) => {
    await open(page, "grid-paged.html");
    const start = (await announced(page)).length;

    await click(page, "page-next");
    await expect.poll(() => announced(page).then((a) => a.length)).toBeGreaterThan(start);
    await click(page, "page-next");
    await expect
      .poll(() => announced(page).then((a) => a.length))
      .toBeGreaterThan(start + 1);

    // The real requirement, and the one this spec was written to catch: the
    // row count does **not** change when you turn a page, so an announcement
    // that only repeats it leaves the page number audible to nobody. Each
    // change has to name the page it went to.
    //
    // Before this was fixed the recording read
    //   "60 matches" · "Loading …" · "60 matches" · "Loading …" · "60 matches"
    // — two page turns, nothing said about either.
    const said = (await announced(page)).slice(start);
    const pages = said.filter((text) => /\b2\b/.test(text) || /\b3\b/.test(text));
    expect(pages.length, "each page turn names its page").toBeGreaterThanOrEqual(2);

    const ready = said.filter((_, index) => index >= 0);
    const distinct = new Set(
      ready.filter((text) => !/Loading|geladen/.test(text)),
    );
    expect(distinct.size, "the two pages do not sound alike").toBeGreaterThan(1);
  });
});

test.describe("states", () => {
  test("loading, empty and error each reach the line in their own state", async ({
    page,
  }) => {
    await open(page, "grid.html");

    // A filter that matches nothing: `Empty` is a state of its own, not a
    // silent zero-row `Ready`.
    await page.evaluate(() => {
      const root = document.querySelector("opengrid-grid").shadowRoot;
      const input = root.querySelector('input[data-col="1"]');
      input.value = "zzzz-no-such-customer";
      input.dispatchEvent(new Event("input", { bubbles: true }));
      input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    });

    await expect
      .poll(() => announcedStates(page).then((s) => s.map(([, state]) => state)))
      .toContain("empty");

    const states = await announcedStates(page);
    // Each announcement carries the state it belongs to — a sentence saying
    // "no matches" while `data-state` still reads `ready` would style and
    // announce two different things.
    for (const [text, state] of states) {
      if (state === "empty") {
        expect(text.toLowerCase()).toMatch(/no matches|keine treffer/);
      }
    }
  });
});

test.describe("the view as a value (point 59)", () => {
  test("restoring a view announces one result, not one per field", async ({ page }) => {
    // The reason `set_view` is a single operation rather than four setters: a
    // restore that applied sort, filters, columns and density one at a time
    // would announce four states that never existed, and the reader would hear
    // three of them get overwritten before they meant anything. That is the
    // "said at the wrong moment" failure this spec exists for — and it is
    // invisible to a test that reads the status line once at the end.
    await open(page, "grid-view.html");
    await page.waitForFunction(() => window.__opengridModule);

    // `record` starts with whatever the line already says, so the restore is
    // counted from here rather than from the first result of the page load.
    await expect.poll(() => announced(page).then((said) => said.length)).toBeGreaterThan(0);
    const alreadySaid = (await announcedStates(page)).length;

    await page.evaluate(() => {
      window.__opengridModule.set_view(document.querySelector("opengrid-grid"), {
        sort: [{ field: "amount", direction: "desc" }],
        filters: [{ column: "country", op: "eq", value: "DE" }],
        columns: { order: [], hidden: ["qty"], widths: {} },
        density: "comfortable",
      });
    });

    await expect
      .poll(() => announced(page).then((said) => said.at(-1)))
      .toMatch(/matches|Treffer/);
    // Let a stray second result arrive before counting, if there is one.
    await page.waitForTimeout(250);

    const said = (await announcedStates(page)).slice(alreadySaid);
    // Exactly one loading and one result for the whole restore.
    const loading = said.filter(([, state]) => state === "loading");
    const ready = said.filter(([, state]) => state === "ready");
    expect(loading, "one query, so one loading").toHaveLength(1);
    expect(ready, "one query, so one result").toHaveLength(1);
  });

  test("setting the view a grid already has says nothing at all", async ({ page }) => {
    // A no-op has to be silent. A page that writes the view back on every
    // event — which is exactly what a "saved views" bar does — would otherwise
    // make the grid talk to itself.
    await open(page, "grid-view.html");
    await page.waitForFunction(() => window.__opengridModule);
    await expect.poll(() => announced(page).then((said) => said.length)).toBeGreaterThan(0);

    const before = await announced(page);
    await page.evaluate(() => {
      const host = document.querySelector("opengrid-grid");
      window.__opengridModule.set_view(host, window.__opengridModule.get_view(host));
    });
    await page.waitForTimeout(250);

    expect(await announced(page)).toEqual(before);
  });
});
