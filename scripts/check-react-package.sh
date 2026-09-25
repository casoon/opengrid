#!/usr/bin/env bash
# Runs the **packed** React adapter the way an installing project would (plan
# point 77): a scratch project whose node_modules holds the two unpacked
# tarballs from `just package` and React from the example — nothing from the
# workspace. There the adapter renders on the server, and its types compile
# against src/types.tsx of the example.
#
# That catches what the workspace hides: a file missing from `files`, a peer
# range still reading `workspace:`, an import the tarball cannot resolve.
#
# Usage: bash scripts/check-react-package.sh   (after `just package`)
set -euo pipefail

cd "$(dirname "$0")/.."
root="$PWD"
packed="$root/target/npm-package/package"
adapter="$root/target/npm-package-react/package"
example="$root/examples/react"
work="$root/target/react-packed"

for dir in "$packed" "$adapter"; do
    [[ -f "$dir/package.json" ]] || {
        echo "no packed package at $dir — run \`just package\` first" >&2
        exit 1
    }
done
if grep -q '"workspace:' "$adapter/package.json"; then
    echo "the packed adapter still names the workspace protocol:" >&2
    grep '"workspace:' "$adapter/package.json" >&2
    exit 1
fi

rm -rf "$work"
mkdir -p "$work/node_modules/@casoon" "$work/node_modules/@types"
ln -s "$packed" "$work/node_modules/@casoon/opengrid"
ln -s "$adapter" "$work/node_modules/@casoon/opengrid-react"
for name in react react-dom; do
    ln -s "$(cd "$example/node_modules/$name" && pwd -P)" "$work/node_modules/$name"
    ln -s "$(cd "$example/node_modules/@types/$name" && pwd -P)" "$work/node_modules/@types/$name"
done
# What @types/react itself depends on, from beside it in the pnpm store.
types_store="$(cd "$example/node_modules/@types/react" && pwd -P)/../.."
ln -s "$(cd "$types_store/csstype" && pwd -P)" "$work/node_modules/csstype"
echo '{ "type": "module" }' > "$work/package.json"

# On the server.
cp "$example/ssr.mjs" "$work/ssr.mjs"
html="$(node --preserve-symlinks "$work/ssr.mjs")"
expected='<opengrid-grid label="Orders" datasource="orders" columns="id,customer" window-size="40" selection="" class="orders"></opengrid-grid>'
if [[ "$html" != "$expected" ]]; then
    echo "the packed adapter rendered something else:" >&2
    echo "  got:      $html" >&2
    echo "  expected: $expected" >&2
    exit 1
fi

# The types.
cp "$example/src/types.tsx" "$work/types.tsx"
cat > "$work/tsconfig.json" <<'JSON'
{
  "compilerOptions": {
    "strict": true,
    "noEmit": true,
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "Bundler",
    "jsx": "react-jsx",
    "lib": ["ES2022", "DOM", "DOM.Iterable"],
    "types": [],
    "preserveSymlinks": true
  },
  "files": ["types.tsx"]
}
JSON
"$root/node_modules/.bin/tsc" -p "$work"
echo "packed React adapter: ok — server rendering and types ($adapter)"
