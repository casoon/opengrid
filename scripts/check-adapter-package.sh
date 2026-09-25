#!/usr/bin/env bash
# Runs a **packed** framework adapter the way an installing project would
# (plan points 77, 78): a scratch project whose node_modules holds copies of
# the unpacked tarballs from `just package` — the element package and the
# adapter — and the framework from the example, nothing from the workspace.
# There the adapter renders on the server exactly as it does in the
# workspace, and its types compile against the example's type test.
#
# That catches what the workspace hides: a file missing from `files`, a peer
# range still reading `workspace:`, an import the tarball cannot resolve.
#
# Usage: bash scripts/check-adapter-package.sh react|vue   (after `just package`)
set -euo pipefail

cd "$(dirname "$0")/.."
root="$PWD"
adapter="${1:?usage: check-adapter-package.sh <react|vue>}"
packed="$root/target/npm-package/package"
packed_adapter="$root/target/npm-package-$adapter/package"
example="$root/examples/$adapter"
work="$root/target/$adapter-packed"

for dir in "$packed" "$packed_adapter"; do
    [[ -f "$dir/package.json" ]] || {
        echo "no packed package at $dir — run \`just package\` first" >&2
        exit 1
    }
done
if grep -q '"workspace:' "$packed_adapter/package.json"; then
    echo "the packed adapter still names the workspace protocol:" >&2
    grep '"workspace:' "$packed_adapter/package.json" >&2
    exit 1
fi

rm -rf "$work"
mkdir -p "$work/node_modules/@casoon"
# Copies, not links: the adapter must resolve its framework from where an
# installed package sits — inside node_modules — and a link would resolve
# from the tarball's own directory instead.
cp -R "$packed" "$work/node_modules/@casoon/opengrid"
cp -R "$packed_adapter" "$work/node_modules/@casoon/opengrid-$adapter"
# Everything else the example has installed, as the store's real directories
# (so what those packages depend on resolves beside them).
link() {
    mkdir -p "$(dirname "$work/node_modules/$2")"
    ln -s "$(cd "$1" && pwd -P)" "$work/node_modules/$2"
}
for entry in "$example"/node_modules/*; do
    name="$(basename "$entry")"
    case "$name" in
        .* | @casoon) continue ;;
        @*) for scoped in "$entry"/*; do link "$scoped" "$name/$(basename "$scoped")"; done ;;
        *) link "$entry" "$name" ;;
    esac
done
echo '{ "type": "module" }' > "$work/package.json"

# On the server: the same HTML as in the workspace.
cp "$example/ssr.mjs" "$work/ssr.mjs"
expected="$(cd "$example" && node ssr.mjs)"
html="$(cd "$work" && node ssr.mjs)"
if [[ "$html" != "$expected" ]]; then
    echo "the packed adapter rendered something else:" >&2
    echo "  packed:    $html" >&2
    echo "  workspace: $expected" >&2
    exit 1
fi

# The types.
types="$(ls "$example"/src/types.ts* | head -1)"
cp "$types" "$work/$(basename "$types")"
cat > "$work/tsconfig.json" <<JSON
{
  "compilerOptions": {
    "strict": true,
    "noEmit": true,
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "Bundler",
    "jsx": "react-jsx",
    "lib": ["ES2022", "DOM", "DOM.Iterable"],
    "types": []
  },
  "files": ["$(basename "$types")"]
}
JSON
"$root/node_modules/.bin/tsc" -p "$work"
echo "packed $adapter adapter: ok — server rendering and types ($packed_adapter)"
