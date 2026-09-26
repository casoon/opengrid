import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// Facets (plan point 66). F4, decided 2026-09-24: always count.
//
// The fixture configures customer as a list, country as pills, amount as a
// range and ordered_on as a period, over the 200 conformance rows. The rule
// this spec exists for is the one that can break without anybody seeing it:
// a facet counts **without its own restriction**.

async function status(page) {
  return page.evaluate(() =>
    document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="status"]').textContent.trim(),
  );
}

/** A facet's values and counts, as drawn: `{ Alpha: 21, … }`. */
async function counts(page, column) {
  return page.evaluate((column) => {
    const side = document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="facets"]');
    const out = {};
    for (const item of side.querySelectorAll(`fieldset[data-facet="${column}"] [part="facet-value"], fieldset[data-facet="${column}"] [part="facet-pill"]`)) {
      const label = item.querySelector("span:not([part])").textContent;
      out[label] = Number(item.querySelector('[part="facet-count"]').textContent);
    }
    return out;
  }, column);
}

async function tick(page, column, key) {
  await page.evaluate(
    ({ column, key }) => {
      const root = document.querySelector("opengrid-grid").shadowRoot;
      const box = [...root.querySelectorAll(`input[data-facet="${column}"][data-key]`)].find(
        (input) => input.dataset.key === key,
      );
      box.checked = !box.checked;
      box.dispatchEvent(new Event("change", { bubbles: true, composed: true }));
    },
    { column, key },
  );
}

async function bound(page, column, which, value) {
  await page.evaluate(
    ({ column, which, value }) => {
      const input = document
        .querySelector("opengrid-grid")
        .shadowRoot.querySelector(`input[data-facet="${column}"][data-bound="${which}"]`);
      input.value = value;
      input.dispatchEvent(new Event("change", { bubbles: true, composed: true }));
    },
    { column, which, value },
  );
}

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid-facets.html");
  await page.waitForFunction(() => window.__opengridReady && window.__opengridModule);
  await expect.poll(() => status(page)).toBe("200 matches");
  await expect.poll(async () => (await counts(page, "customer")).Alpha).toBe(21);
});

test("a facet counts without its own restriction", async ({ page }) => {
  // After ticking Alpha, Beta still counts its 29 rows — or the facet could
  // only be undone, never used. The *other* facets count within Alpha.
  await tick(page, "customer", '"Alpha"');
  await expect.poll(() => status(page)).toBe("21 matches");
  await expect.poll(async () => (await counts(page, "country")).DE).toBe(6);

  const customers = await counts(page, "customer");
  expect(customers.Beta).toBe(29);
  expect(customers.Alpha).toBe(21);
  const countries = await counts(page, "country");
  expect(countries.DE + countries.FR + countries.GB + countries.US + countries["(no value)"]).toBe(21);
});

test("one query per facet column, not one per value", async ({ page }) => {
  await page.evaluate(() => {
    window.__queries.length = 0;
  });
  await tick(page, "customer", '"Beta"');
  await expect.poll(() => status(page)).toBe("29 matches");
  await page.waitForTimeout(300);
  const groups = await page.evaluate(() =>
    window.__queries.map((q) => JSON.parse(q)).filter((q) => q.group).map((q) => q.group[0]),
  );
  // customer and country — the two counted facets — once each; nine customers
  // and five countries would have been fourteen.
  expect(groups.sort()).toEqual(["country", "customer"]);
  expect(
    await page.evaluate(
      () => document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="facet-cost"]').textContent,
    ),
  ).toBe("Counted with 2 queries");
});

test("NULL is a value with a name, and ticking it finds the rows without one", async ({ page }) => {
  // `eq null` would match nothing (S1); the facet asks `is_null`.
  expect((await counts(page, "customer"))["(no value)"]).toBe(3);
  await tick(page, "customer", "null");
  await expect.poll(() => status(page)).toBe("3 matches");
});

test("two values of one facet are an or, two facets an and", async ({ page }) => {
  await tick(page, "customer", '"Alpha"');
  await tick(page, "customer", '"Beta"');
  await expect.poll(() => status(page)).toBe("50 matches");
  await page.evaluate(() =>
    [...document.querySelector("opengrid-grid").shadowRoot.querySelectorAll('[part="facet-pill"]')]
      .find((pill) => pill.dataset.key === '"DE"')
      .click(),
  );
  await expect.poll(async () => Number((await status(page)).split(" ")[0])).toBeLessThan(50);
  expect(
    await page.evaluate(() =>
      [...document.querySelector("opengrid-grid").shadowRoot.querySelectorAll('[part="facet-pill"]')]
        .find((pill) => pill.dataset.key === '"DE"')
        .getAttribute("aria-pressed"),
    ),
  ).toBe("true");
});

test("a range and a period restrict by their bounds", async ({ page }) => {
  await bound(page, "amount", "low", "900000000");
  await expect.poll(async () => Number((await status(page)).split(" ")[0])).toBeLessThan(200);
  const high = Number((await status(page)).split(" ")[0]);
  await bound(page, "ordered_on", "high", "2026-01-01");
  await expect.poll(async () => Number((await status(page)).split(" ")[0])).toBeLessThan(high);
});

test("a bound that is not a value is named, not half-applied", async ({ page }) => {
  await bound(page, "amount", "low", "abc");
  await expect.poll(() => status(page)).toContain("abc");
});

test("every bound has a visible label, not only a placeholder", async ({ page }) => {
  // 3.3.2: a placeholder is gone the moment someone types.
  const labels = await page.evaluate(() =>
    [...document.querySelector("opengrid-grid").shadowRoot.querySelectorAll("input[data-bound]")].map((input) => ({
      inLabel: !!input.closest("label"),
      text: input.closest("label")?.textContent.trim(),
      legend: input.closest("fieldset")?.querySelector("legend")?.textContent,
    })),
  );
  expect(labels.length).toBe(4);
  for (const label of labels) {
    expect(label.inLabel).toBe(true);
    expect(["From", "To"]).toContain(label.text);
    expect(["amount", "ordered_on"]).toContain(label.legend);
  }
});

test("a facet is a chip, and removing the chip clears the facet", async ({ page }) => {
  await tick(page, "customer", '"Alpha"');
  await tick(page, "customer", '"Beta"');
  await expect.poll(() => status(page)).toBe("50 matches");
  const chip = await page.evaluate(
    () => document.querySelector("opengrid-grid").shadowRoot.querySelector('[data-chip-remove="facet:customer"]').getAttribute("aria-label"),
  );
  expect(chip).toBe("Remove customer is one of Alpha, Beta");
  await page.evaluate(() =>
    document.querySelector("opengrid-grid").shadowRoot.querySelector('[data-chip-remove="facet:customer"]').click(),
  );
  await expect.poll(() => status(page)).toMatch(/^200 matches/);
});

test("Reset clears every facet", async ({ page }) => {
  await tick(page, "customer", '"Alpha"');
  await bound(page, "amount", "low", "1");
  await expect.poll(() => status(page)).not.toBe("200 matches");
  await page.evaluate(() =>
    document.querySelector("opengrid-grid").shadowRoot.querySelector("[data-facets-reset]").click(),
  );
  await expect.poll(() => status(page)).toBe("200 matches");
});

test("the facets travel in the view and come back", async ({ page }) => {
  await tick(page, "customer", "null");
  await bound(page, "amount", "low", "5");
  await expect.poll(async () => (await status(page)).split(" ")[0]).not.toBe("200");
  const view = await page.evaluate(() => window.__opengridModule.get_view(document.querySelector("opengrid-grid")));
  expect(view.facets).toEqual({ customer: { values: [null] }, amount: { min: "5", max: "" } });

  await page.evaluate(() => document.querySelector("opengrid-grid").shadowRoot.querySelector("[data-facets-reset]").click());
  await expect.poll(() => status(page)).toBe("200 matches");
  await page.evaluate((view) => window.__opengridModule.set_view(document.querySelector("opengrid-grid"), view), view);
  await expect.poll(() => status(page)).not.toBe("200 matches");
  expect(
    await page.evaluate(
      () => [...document.querySelector("opengrid-grid").shadowRoot.querySelectorAll('input[data-facet="customer"]')].find((box) => box.dataset.key === "null").checked,
    ),
  ).toBe(true);
});

test("the keyboard reaches and ticks a facet", async ({ page }) => {
  // The grid's keys stop at the sidebar: a checkbox keeps its native Space.
  await page.evaluate(() =>
    [...document.querySelector("opengrid-grid").shadowRoot.querySelectorAll('input[data-facet="customer"]')]
      .find((box) => box.dataset.key === '"Gamma"')
      .focus(),
  );
  await page.keyboard.press(" ");
  await expect.poll(() => status(page)).toBe("26 matches");
});

test("the facet switch hides the sidebar and gives its width back", async ({ page }) => {
  const width = () =>
    page.evaluate(() => document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="viewport"]').getBoundingClientRect().width);
  const before = await width();
  await page.evaluate(() => document.querySelector("opengrid-grid").shadowRoot.querySelector('[data-toolbar="facets"]').click());
  await expect.poll(() => page.evaluate(() => document.querySelector("opengrid-grid").hasAttribute("facets"))).toBe(false);
  await expect.poll(width).toBeGreaterThan(before);
});

test("Tab meets the controls and the grid, never a scroller", async ({ page, browserName }) => {
  // Firefox makes every scroll container a tab stop of its own, focusable
  // children or not — an unnamed stop that says nothing. Here the filter row,
  // the facets and the viewport all scroll, so each would be one.
  const scrolling = await page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    return ["filter", "facets", "viewport"].filter((part) => {
      const box = root.querySelector(`[part="${part}"]`);
      return box.scrollWidth > box.clientWidth || box.scrollHeight > box.clientHeight;
    });
  });
  expect(scrolling).toEqual(["filter", "facets", "viewport"]);

  // WebKit on macOS keeps Safari's default: Tab skips buttons, Option+Tab
  // reaches every control (as in grid.spec.js).
  const tab = browserName === "webkit" ? "Alt+Tab" : "Tab";
  await page.evaluate(() =>
    document.querySelector("opengrid-grid").shadowRoot.querySelector("button, select, input").focus(),
  );
  const stops = [];
  let arrived = false;
  for (let step = 0; step < 200; step += 1) {
    const stop = await page.evaluate(() => {
      const outer = document.activeElement;
      if (outer?.id === "after") return "after";
      if (outer !== document.querySelector("opengrid-grid")) return `outside:${outer?.tagName}`;
      const inner = outer.shadowRoot.activeElement;
      const part = inner.getAttribute("part");
      const facet = inner.hasAttribute("data-facet") ? "[facet]" : "";
      return `${inner.tagName.toLowerCase()}${part ? `[part=${part}]` : ""}${facet}`;
    });
    if (stop === "after") {
      arrived = true;
      break;
    }
    stops.push(stop);
    if (stop.startsWith("outside:")) break;
    await page.keyboard.press(tab);
  }
  // The walk went through the filter row, the facets and into the grid …
  expect(stops).toContain("input[part=filter-value]");
  expect(stops.some((stop) => stop.endsWith("[facet]"))).toBe(true);
  expect(stops.some((stop) => stop.startsWith("th") || stop.startsWith("td"))).toBe(true);
  // … met not one container, and left the grid for the next control: a walk
  // that never gets out is a keyboard trap, not a pass.
  expect(stops.filter((stop) => stop.startsWith("div") || stop.startsWith("outside:"))).toEqual([]);
  expect(arrived).toBe(true);
});

test("has no axe violations with all four kinds of facet", async ({ page }) => {
  await tick(page, "customer", '"Alpha"');
  await expect.poll(() => status(page)).toBe("21 matches");
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
});
