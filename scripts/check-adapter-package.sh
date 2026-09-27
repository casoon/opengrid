#!/usr/bin/env bash
# Runs a **packed** framework adapter the way an installing project would
# (plan points 77–79, #27): a scratch project whose node_modules holds a copy
# of the unpacked tarball from `just package` — the adapters are its subpaths
# `@casoon/opengrid/react|vue|svelte` — and the framework from the example,
# nothing from the workspace.
# There the adapter renders on the server exactly as it does in the
# workspace, and its types compile against the example's type test.
#
# That catches what the workspace hides: a file missing from `files`, a
# subpath missing from `exports`, an import the tarball cannot resolve.
#
# Usage: bash scripts/check-adapter-package.sh react|vue|svelte   (after `just package`)
set -euo pipefail

cd "$(dirname "$0")/.."
root="$PWD"
adapter="${1:?usage: check-adapter-package.sh <react|vue|svelte>}"
packed="$root/target/npm-package/package"
example="$root/examples/$adapter"
work="$root/target/$adapter-packed"

[[ -f "$packed/package.json" ]] || {
    echo "no packed package at $packed — run \`just package\` first" >&2
    exit 1
}

rm -rf "$work"
mkdir -p "$work/node_modules/@casoon"
# Copies, not links: the adapter must resolve its framework from where an
# installed package sits — inside node_modules — and a link would resolve
# from the tarball's own directory instead.
cp -R "$packed" "$work/node_modules/@casoon/opengrid"
# The framework, named per adapter — its peers, the types they need, and what
# rendering on the server takes — linked from the example as the store's real
# directories, so what those depend on resolves beside them. Nothing else: an
# import the adapter does not declare must fail here, not find a package the
# example happens to have.
case "$adapter" in
    react) framework=(react react-dom @types/react @types/react-dom) ;;
    vue) framework=(vue) ;;
    svelte) framework=(svelte vite @sveltejs/vite-plugin-svelte) ;;
    *) echo "unknown adapter: $adapter" >&2; exit 1 ;;
esac
for name in "${framework[@]}"; do
    source_dir="$example/node_modules/$name"
    [[ -d "$source_dir" ]] || {
        echo "$name is not installed in $example — run \`pnpm install\`" >&2
        exit 1
    }
    mkdir -p "$(dirname "$work/node_modules/$name")"
    ln -s "$(cd "$source_dir" && pwd -P)" "$work/node_modules/$name"
done
echo '{ "type": "module" }' > "$work/package.json"

# On the server: the same HTML as in the workspace.
cp "$example/ssr.mjs" "$work/ssr.mjs"
mkdir -p "$work/src"
cp "$example/src/ssr-props.js" "$work/src/ssr-props.js"
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
echo "packed $adapter adapter: ok — server rendering and types (@casoon/opengrid/$adapter)"
