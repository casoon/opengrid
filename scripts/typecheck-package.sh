#!/usr/bin/env bash
# Type-checks tests/types/api.ts against the **packed** package (plan point 75).
#
# The repository check (`tsc -p tests/types`) maps `@casoon/opengrid` straight
# to packages/opengrid/loader.d.ts. That proves the types, not the package: a
# `.d.ts` missing from `files` would pass there and fail for every user. So the
# same file is compiled here in a scratch project whose node_modules holds the
# unpacked tarball from `just package`, resolved through its package.json like
# any installed dependency — once as a bundler resolves, once as Node does
# (NodeNext). What this does not hold is the shape of `exports` for the root:
# TypeScript finds `loader.d.ts` beside `loader.js` either way. packaged.spec.js
# does. tests/types/engine.ts is compiled here only: it imports the engine the
# package ships through `@casoon/opengrid/engine/…`, a subpath that resolves
# through `exports` or not at all.
#
# Usage: bash scripts/typecheck-package.sh   (after `just package`)
set -euo pipefail

cd "$(dirname "$0")/.."
root="$PWD"
packed="$root/target/npm-package/package"
work="$root/target/typecheck"

if [[ ! -f "$packed/package.json" ]]; then
    echo "no packed package at $packed — run \`just package\` first" >&2
    exit 1
fi

rm -rf "$work"
mkdir -p "$work/node_modules/@casoon"
ln -s "$packed" "$work/node_modules/@casoon/opengrid"
cp "$root/tests/types/api.ts" "$work/api.ts"
cp "$root/tests/types/engine.ts" "$work/engine.ts"
# An ES module, as a page's code is; under NodeNext a `.ts` file is CommonJS
# unless its package says otherwise.
echo '{ "type": "module" }' > "$work/package.json"
# `ESNext.Disposable`: the engine's generated types (wasm-bindgen) declare
# `[Symbol.dispose]()` on `Engine` and `Planner`. A project that checks library
# types needs that lib, as docs/api.md says; without it they do not compile.
cat > "$work/tsconfig.json" <<'JSON'
{
  "compilerOptions": {
    "strict": true,
    "noEmit": true,
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "Bundler",
    "lib": ["ES2022", "ESNext.Disposable", "DOM", "DOM.Iterable"],
    "types": [],
    "preserveSymlinks": true
  },
  "files": ["api.ts", "engine.ts"]
}
JSON

"$root/node_modules/.bin/tsc" -p "$work"
"$root/node_modules/.bin/tsc" -p "$work" --module NodeNext --moduleResolution NodeNext
echo "packed types: ok, bundler and NodeNext ($packed)"
