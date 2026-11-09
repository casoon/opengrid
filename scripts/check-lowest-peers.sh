#!/usr/bin/env bash
# Type-checks a **packed** framework adapter against the *lowest* version of its
# peer that package.json allows (#237). `check-adapter-package.sh` uses the
# framework the example has, which is the newest; a type the adapter's `.d.ts`
# imports from a newer release (`DefineSetupFnComponent` is Vue >= 3.4.20) would
# pass there and fail for a user on the declared minimum.
#
# The lowest versions are installed from the registry into a scratch project
# (one small download per framework), so this runs from `just types` but needs
# the network. Keep the versions in step with `peerDependencies`.
#
# Usage: bash scripts/check-lowest-peers.sh react|vue|svelte   (after `just package`)
set -euo pipefail

cd "$(dirname "$0")/.."
root="$PWD"
adapter="${1:?usage: check-lowest-peers.sh <react|vue|svelte>}"
packed="$root/target/npm-package/package"
example="$root/examples/$adapter"
work="$root/target/$adapter-lowest"

[[ -f "$packed/package.json" ]] || {
    echo "no packed package at $packed — run \`just package\` first" >&2
    exit 1
}

case "$adapter" in
    # @types/react before 18.3 still imports `scheduler/tracing`, which the
    # current @types/scheduler no longer has: that fails on its own, not in ours.
    react) lowest=(react@18.2.0 @types/react@18.3.0) ;;
    vue) lowest=(vue@3.4.20) ;;
    svelte) lowest=(svelte@5.0.0) ;;
    *) echo "unknown adapter: $adapter" >&2; exit 1 ;;
esac

rm -rf "$work"
mkdir -p "$work"
echo '{ "private": true, "type": "module" }' > "$work/package.json"
(cd "$work" && npm install --no-save --no-package-lock --ignore-scripts --no-audit --no-fund "${lowest[@]}" >/dev/null)
# After the install: npm prunes what it does not know from node_modules.
mkdir -p "$work/node_modules/@casoon"
cp -R "$packed" "$work/node_modules/@casoon/opengrid"

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
    "types": [],
    "skipLibCheck": false
  },
  "files": ["$(basename "$types")"]
}
JSON
"$root/node_modules/.bin/tsc" -p "$work"
echo "lowest peers: ok — ${lowest[*]} (@casoon/opengrid/$adapter)"
