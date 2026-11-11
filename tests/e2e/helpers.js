import { inflateRawSync } from "node:zlib";

import { expect } from "@playwright/test";

// What the specs share, as far as their copies were byte for byte the same.
//
// `shadow` runs a function against the shadow root of the page's first
// `<opengrid-grid>` and hands it one argument; `status` reads the status line.
// The specs that differ — another element, a selector, another return — keep
// their own: a helper that took every variant's options would be harder to read
// than the copies it replaced (#361).

export const shadow = (page, fn, arg) =>
  page.evaluate(
    ({ fn, arg }) => new Function("root", "arg", `return (${fn})(root, arg)`)(
      document.querySelector("opengrid-grid").shadowRoot,
      arg,
    ),
    { fn: fn.toString(), arg },
  );

export const status = (page) => shadow(page, (root) => root.querySelector('[part="status"]').textContent);

/** The files of a ZIP archive by name, read from its central directory. */
export function unzip(file) {
  const end = file.lastIndexOf(Buffer.from([0x50, 0x4b, 0x05, 0x06]));
  expect(end, "a ZIP archive ends in its directory").toBeGreaterThan(-1);
  const files = new Map();
  let at = file.readUInt32LE(end + 16);
  for (let entry = file.readUInt16LE(end + 10); entry > 0; entry--) {
    const method = file.readUInt16LE(at + 10);
    const size = file.readUInt32LE(at + 20);
    const nameLength = file.readUInt16LE(at + 28);
    const local = file.readUInt32LE(at + 42);
    const name = file.toString("utf8", at + 46, at + 46 + nameLength);
    const start = local + 30 + file.readUInt16LE(local + 26) + file.readUInt16LE(local + 28);
    const data = file.subarray(start, start + size);
    files.set(name, method === 0 ? data : inflateRawSync(data));
    at += 46 + nameLength + file.readUInt16LE(at + 30) + file.readUInt16LE(at + 32);
  }
  return files;
}
