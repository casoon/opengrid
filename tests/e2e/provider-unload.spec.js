import { test, expect } from "@playwright/test";

// `unload(name)` on the engine providers (issue #459): one source goes, the others stay, a query on
// it is an unknown source, and a load after it takes the room the table had instead of adding to it.

async function open(page) {
  await page.goto("/tests/e2e/fixtures/grid-rows.html");
  await page.waitForFunction(() => window.__ready === true);
}

test("the local provider frees one source and can load it again", async ({ page }) => {
  await open(page);
  const result = await page.evaluate(async () => {
    const provider = window.__provider;
    await provider.loadStream("other", new Blob(["id,name,born\n9,Z,\\N\n"]), window.__schema);
    const first = await provider.unload("people");
    const second = await provider.unload("people");
    const names = (await provider.stats()).sources.map((source) => source.name);
    let error = null;
    try {
      await provider.execute(JSON.stringify({ source: "people", select: ["id"] }));
    } catch (thrown) {
      error = String(thrown?.message ?? thrown);
    }
    await provider.loadStream("people", new Blob(["id,name,born\n1,A,\\N\n"]), window.__schema);
    const again = (await provider.stats()).sources.find((source) => source.name === "people");
    return { first, second, names, error, rows: again?.rows };
  });
  expect(result.first).toBe(true);
  expect(result.second).toBe(false);
  expect(result.names).toEqual(["other"]);
  expect(result.error).toContain("people");
  expect(result.rows).toBe(1);
});

test("the worker frees a source, and a load after it does not add to the memory", async ({ page }) => {
  await open(page);
  const result = await page.evaluate(async (schema) => {
    const { createWorkerProvider } = await import("/packages/opengrid/loader.js");
    const lines = ["id,name,born"];
    for (let i = 1; i <= 100_000; i++) lines.push(`${i},Name ${i % 500},2001-02-03`);
    const file = lines.join("\n");
    const memory = async (provider) => (await provider.stats()).memory;

    // Loaded twice under one name: both tables are there for a moment.
    const kept = createWorkerProvider();
    await kept.loadStream("people", new Blob([file]), schema);
    await kept.loadStream("people", new Blob([file]), schema);
    const twice = await memory(kept);
    kept.terminate();

    // Unloaded in between: the second load takes the first one's room.
    const freed = createWorkerProvider();
    await freed.loadStream("people", new Blob([file]), schema);
    const once = await memory(freed);
    const first = await freed.unload("people");
    const empty = (await freed.stats()).sources.length;
    let error = null;
    try {
      await freed.execute(JSON.stringify({ source: "people", select: ["id"] }));
    } catch (thrown) {
      error = String(thrown?.message ?? thrown);
    }
    await freed.loadStream("people", new Blob([file]), schema);
    const after = await memory(freed);
    const second = await freed.unload("nobody");
    freed.terminate();
    return { twice, once, after, first, second, empty, error };
  }, await page.evaluate(() => window.__schema));
  expect(result.first).toBe(true);
  expect(result.second).toBe(false);
  expect(result.empty).toBe(0);
  expect(result.error).toContain("people");
  expect(result.after).toBe(result.once);
  expect(result.after).toBeLessThan(result.twice);
});
