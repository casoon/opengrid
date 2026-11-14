import { test, expect } from "@playwright/test";
import { shadow } from "./helpers.js";

// A CSV file read a piece at a time (issue #427): the same source as the whole file, from a Blob
// or a stream, with the pieces cut anywhere; a file that cannot be read leaves the old source.

const names = (page) =>
  shadow(page, (root) =>
    [...root.querySelectorAll("tbody tr")]
      .filter((tr) => tr.querySelector('td[data-col="0"]')?.textContent.trim())
      .map((tr) => tr.querySelector('td[data-col="1"]').textContent));

async function open(page) {
  await page.goto("/tests/e2e/fixtures/grid-rows.html");
  await page.waitForFunction(() => window.__ready === true);
  await expect.poll(() => names(page)).toEqual(["Ada", "Grace", "Edsger"]);
}

const FILE = 'id,name,born\n1,"Ünal\nKemal",1900-01-02\n2,Zoë,\\N\n3,"say ""hi""",2001-02-03\n';

test("a Blob in tiny pieces is the source, and the grid shows it", async ({ page }) => {
  await open(page);
  await page.evaluate(
    (file) =>
      window.__provider.loadStream("people", new Blob([file]), window.__schema, { chunkSize: 5 }),
    FILE,
  );
  await expect
    .poll(() => names(page))
    .toEqual(["Ünal\nKemal", "Zoë", 'say "hi"']);
});

test("a stream cut inside a character is the same source", async ({ page }) => {
  await open(page);
  await page.evaluate(async (file) => {
    const bytes = new TextEncoder().encode(file);
    // One byte per piece: every multi-byte character is cut.
    const stream = new ReadableStream({
      start(controller) {
        for (let at = 0; at < bytes.length; at++) controller.enqueue(bytes.slice(at, at + 1));
        controller.close();
      },
    });
    await window.__provider.loadStream("people", stream, window.__schema);
  }, FILE);
  await expect.poll(() => names(page)).toEqual(["Ünal\nKemal", "Zoë", 'say "hi"']);
  const stats = await page.evaluate(() => window.__provider.stats());
  expect(stats.sources.find((source) => source.name === "people").rows).toBe(3);
});

test("a file that cannot be read rejects with its line and the old rows stay", async ({ page }) => {
  await open(page);
  const message = await page.evaluate(() =>
    window.__provider
      .loadStream("people", new Blob(["id,name,born\n1,A,\\N\n2\n"]), window.__schema, {
        chunkSize: 4,
      })
      .then(() => null, (error) => error.message),
  );
  expect(message).toContain("line 3");
  await expect.poll(() => names(page)).toEqual(["Ada", "Grace", "Edsger"]);
});

test("the worker reads the pieces the same way, and drops a file that fails", async ({ page }) => {
  await open(page);
  const result = await page.evaluate(
    async ({ file, schema }) => {
      const { createWorkerProvider } = await import("/packages/opengrid/loader.js");
      const worker = createWorkerProvider();
      await worker.loadStream("people", new Blob([file]), schema, { chunkSize: 3 });
      const loaded = (await worker.stats()).sources.find((s) => s.name === "people").rows;
      const message = await worker
        .loadStream("people", new Blob(["id,name,born\n1,A,\\N\n2\n"]), schema, { chunkSize: 4 })
        .then(() => null, (error) => error.message);
      // The failed file registered nothing and left nothing begun: the next one loads.
      await worker.loadStream("people", new Blob(["id,name,born\n9,Z,\\N\n"]), schema);
      const after = (await worker.stats()).sources.find((s) => s.name === "people").rows;
      worker.terminate();
      return { loaded, message, after };
    },
    { file: FILE, schema: await page.evaluate(() => window.__schema) },
  );
  expect(result.loaded).toBe(3);
  expect(result.message).toContain("line 3");
  expect(result.after).toBe(1);
});

test("an async iterable that reuses one buffer is read as it was delivered", async ({ page }) => {
  await open(page);
  const rows = await page.evaluate(
    async ({ file, schema }) => {
      const { createWorkerProvider } = await import("/packages/opengrid/loader.js");
      const worker = createWorkerProvider();
      const bytes = new TextEncoder().encode(file);
      // One buffer for every piece, refilled each time: the loader must not hand it on as its own
      // (a transferred buffer is detached, and the next refill throws).
      const reused = new Uint8Array(4);
      async function* pieces() {
        for (let at = 0; at < bytes.length; at += 4) {
          const piece = bytes.subarray(at, at + 4);
          reused.set(piece);
          yield reused.subarray(0, piece.length);
        }
      }
      await worker.loadStream("people", pieces(), schema);
      const rows = (await worker.stats()).sources.find((s) => s.name === "people").rows;
      worker.terminate();
      return rows;
    },
    { file: FILE, schema: await page.evaluate(() => window.__schema) },
  );
  expect(rows).toBe(3);
});

test("a source that is not a source is refused, and the old rows stay", async ({ page }) => {
  await open(page);
  const message = await page.evaluate(() =>
    window.__provider
      .loadStream("people", "id,name,born\n1,A,\\N\n", window.__schema)
      .then(() => null, (error) => error.message),
  );
  expect(message).toContain("Blob");
  await expect.poll(() => names(page)).toEqual(["Ada", "Grace", "Edsger"]);
});
