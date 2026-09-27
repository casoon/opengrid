#!/usr/bin/env bash
# Packs `@casoon/opengrid` the way a release would, and unpacks it for testing.
# The React, Vue and Svelte components are in it as the subpaths /react, /vue
# and /svelte (#27) — one package, one tarball.
#
# The point of this script is that nothing downstream reads the repository. It
# builds the module, stages the files `package.json#files` promises, runs
# `npm pack`, and extracts the resulting tarball to `target/npm-package/package`
# — which is what tests/e2e/packaged.spec.js loads. A file that the `files` list
# forgets is therefore a failing test, not a surprise after publishing.
#
# Usage: bash scripts/pack-npm.sh
set -euo pipefail

cd "$(dirname "$0")/.."
root="$PWD"
pkg="$root/packages/opengrid"
dest="$root/target/npm-package"

# The modules the package ships. Built here so `files` has something to find:
# the element module into pkg/, the engine (`Engine`, `Planner`) into engine/ —
# the default of `createWorkerProvider()`, and what a page imports to query in
# the tab. Both directories are build output and gitignored.
#
# Emptied first: wasm-bindgen writes into a directory and never deletes, and
# `files` ships whatever is there — a snippet directory left from an older
# build would go out with the release. packaged.spec.js checks the snippets.
rm -rf "$pkg/pkg" "$pkg/engine"
just wasm-build-components
just wasm-build packages/opengrid/engine

# The tarball is only what CI checked if the same wasm-opt built it: CI pins
# binaryen (`BINARYEN_VERSION` in .github/workflows/ci.yml), a machine has
# whatever it installed. A warning, not a failure — packing locally to read the
# contents is fine; publishing takes the tarballs CI packed (docs/releasing.md).
wasm_opt="$(wasm-opt --version)"
pinned="$(sed -n 's/^  BINARYEN_VERSION: "\([0-9]*\)"$/\1/p' "$root/.github/workflows/ci.yml")"
echo "wasm-opt: $wasm_opt (CI pins $pinned)"
if [[ "$wasm_opt" != *"version $pinned "* && "$wasm_opt" != *"version $pinned" ]]; then
    echo "warning: wasm-opt is not binaryen $pinned as CI pins it — this tarball is not the one CI checked" >&2
fi

# Staged, not authored: the licences and the readme live at the repository root
# and are copied in for the tarball. Removed again below so the working tree
# keeps exactly one copy of each.
staged=()
for file in LICENSE-MIT LICENSE-APACHE README.md CHANGELOG.md; do
    cp "$root/$file" "$pkg/$file"
    staged+=("$pkg/$file")
done
cleanup() { rm -f "${staged[@]}"; }
trap cleanup EXIT

rm -rf "$dest"
mkdir -p "$dest"
tarball="$(cd "$pkg" && npm pack --pack-destination "$dest" --silent)"
tar -xzf "$dest/$tarball" -C "$dest"

echo "packed:   $dest/$tarball"
echo "unpacked: $dest/package"
echo
echo "contents:"
tar -tzf "$dest/$tarball" | sort
