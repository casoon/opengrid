#!/usr/bin/env bash
# Measures the WASM modules plan point 40 decides between.
#
# Builds `opengrid-web-components` three ways — both elements, grid only, pivot
# only — through the same pipeline the shipped module uses (`cargo` release →
# `wasm-bindgen --target web` → `wasm-opt -Oz`), then reports raw, gzip and
# brotli for each — and the engine module (`opengrid-wasm`) the package ships
# under engine/, through the same pipeline. brotli comes from `node:zlib` at quality 11 because no
# `brotli` CLI is installed here; that is the same way the engine's numbers in
# plan/spezifikation/12-qualitaet.md §WASM-Größe were taken, so they compare.
#
# Usage: bash scripts/measure-modules.sh
set -euo pipefail

cd "$(dirname "$0")/.."
target="${CARGO_TARGET_DIR:-target}"
out="$(mktemp -d)"
trap 'rm -rf "$out"' EXIT

build() {
    local name="$1" crate="${CRATE:-opengrid-web-components}"
    shift
    cargo build --release --target wasm32-unknown-unknown \
        -p "$crate" "$@" >/dev/null 2>&1
    wasm-bindgen --target web --out-dir "$out/$name" --out-name m \
        "$target/wasm32-unknown-unknown/release/${crate//-/_}.wasm" >/dev/null 2>&1
    wasm-opt -Oz -o "$out/$name/m_bg.wasm" "$out/$name/m_bg.wasm"
}

report() {
    local label="$1" file="$2"
    node -e '
      const fs = require("fs"), zlib = require("zlib");
      const b = fs.readFileSync(process.argv[2]);
      const gz = zlib.gzipSync(b, { level: 9 }).length;
      const br = zlib.brotliCompressSync(b, {
        params: { [zlib.constants.BROTLI_PARAM_QUALITY]: 11 },
      }).length;
      const kib = (n) => (n / 1024).toFixed(1) + " KiB";
      console.log(
        [process.argv[1], b.length, kib(b.length), gz, kib(gz), br, kib(br)].join("\t"),
      );
    ' "$label" "$file"
}

build both
build grid-only --no-default-features --features grid
build pivot-only --no-default-features --features pivot
CRATE=opengrid-wasm build engine

printf 'module\traw_B\traw\tgzip_B\tgzip\tbrotli_B\tbrotli\n'
report "grid+pivot" "$out/both/m_bg.wasm"
report "grid only" "$out/grid-only/m_bg.wasm"
report "pivot only" "$out/pivot-only/m_bg.wasm"
report "engine" "$out/engine/m_bg.wasm"

# What the two would cost side by side, and what they share.
node -e '
  const fs = require("fs"), zlib = require("zlib");
  const br = (p) => zlib.brotliCompressSync(fs.readFileSync(p), {
    params: { [zlib.constants.BROTLI_PARAM_QUALITY]: 11 },
  }).length;
  const raw = (p) => fs.statSync(p).size;
  const [both, g, p] = process.argv.slice(1);
  const kib = (n) => (n / 1024).toFixed(1) + " KiB";
  console.log("");
  console.log("split total (raw)   ", raw(g) + raw(p), kib(raw(g) + raw(p)),
              "vs one module", raw(both), kib(raw(both)));
  console.log("split total (brotli)", br(g) + br(p), kib(br(g) + br(p)),
              "vs one module", br(both), kib(br(both)));
  console.log("shared, i.e. duplicated by a split (raw)   ",
              raw(g) + raw(p) - raw(both), kib(raw(g) + raw(p) - raw(both)));
  console.log("shared, i.e. duplicated by a split (brotli)",
              br(g) + br(p) - br(both), kib(br(g) + br(p) - br(both)));
' "$out/both/m_bg.wasm" "$out/grid-only/m_bg.wasm" "$out/pivot-only/m_bg.wasm"
