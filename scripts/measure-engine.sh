#!/usr/bin/env bash
# What the engine module is made of, by crate (issue #37, decision E34).
#
# Builds `opengrid-wasm` through the shipped pipeline (`cargo` release →
# `wasm-bindgen --target web` → `wasm-opt -Oz`), once more with the function
# names kept (`--keep-debug`, `wasm-opt -g`), and lets `twiggy` attribute every
# function and data segment. Generic code is counted to the first non-`core`,
# non-`alloc` crate in its name, so a `core` sort instantiated for a column type
# lands on the crate that asked for it. The shipped size (raw, brotli) is the
# first line; the names only change the build that is attributed.
#
# Needs `twiggy` (`cargo install twiggy`) and `node` for brotli, as
# `measure-modules` does.
#
# Usage: bash scripts/measure-engine.sh
set -euo pipefail

cd "$(dirname "$0")/.."
target="${CARGO_TARGET_DIR:-target}"
out="$(mktemp -d)"
trap 'rm -rf "$out"' EXIT

# The same path remapping as the shipped build (`REMAP` in the justfile).
RUSTFLAGS="${REMAP:-}" cargo build --release --target wasm32-unknown-unknown -p opengrid-wasm >/dev/null 2>&1
module="$target/wasm32-unknown-unknown/release/opengrid_wasm.wasm"
wasm-bindgen --target web --out-dir "$out/shipped" --out-name m "$module" >/dev/null 2>&1
wasm-opt -Oz -o "$out/shipped/m_bg.wasm" "$out/shipped/m_bg.wasm"
wasm-bindgen --target web --keep-debug --out-dir "$out/named" --out-name m "$module" >/dev/null 2>&1
wasm-opt -Oz -g -o "$out/named/m_bg.wasm" "$out/named/m_bg.wasm"

node -e '
  const fs = require("fs"), zlib = require("zlib");
  const b = fs.readFileSync(process.argv[1]);
  const br = zlib.brotliCompressSync(b, {
    params: { [zlib.constants.BROTLI_PARAM_QUALITY]: 11 },
  }).length;
  console.log(`engine module: ${b.length} B raw, ${br} B brotli (${(br / 1024).toFixed(1)} KiB)\n`);
' "$out/shipped/m_bg.wasm"

twiggy top -n 1000000 --format csv "$out/named/m_bg.wasm" | node -e '
  const lines = require("fs").readFileSync(0, "utf8").trim().split("\n").slice(1);
  const sizes = new Map();
  let total = 0;
  for (const line of lines) {
    const match = line.match(/^(".*"|[^,]*),(\d+),/);
    if (!match) continue;
    const [, name, size] = match;
    if (name.includes("subsection") || name.includes("custom section")) continue;
    const crates = [...name.matchAll(/([a-z_0-9]+)\[[0-9a-f]+\]/g)].map((m) => m[1]);
    const crate =
      crates.find((c) => !["core", "alloc", "std"].includes(c)) ??
      crates[0] ??
      (name.includes("data segment") ? "(data)" : "(other)");
    sizes.set(crate, (sizes.get(crate) ?? 0) + Number(size));
    total += Number(size);
  }
  console.log("crate\tKiB raw\tshare");
  for (const [crate, size] of [...sizes].sort((a, b) => b[1] - a[1])) {
    console.log(`${crate}\t${(size / 1024).toFixed(1)}\t${((100 * size) / total).toFixed(1)} %`);
  }
'
