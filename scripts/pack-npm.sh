#!/usr/bin/env bash
# Packs `@casoon/opengrid` the way a release would, and unpacks it for testing.
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
# The framework adapters (points 77–79) ship the licences too.
adapters=(react vue svelte)
for adapter in "${adapters[@]}"; do
    for file in LICENSE-MIT LICENSE-APACHE; do
        cp "$root/$file" "$root/packages/opengrid-$adapter/$file"
        staged+=("$root/packages/opengrid-$adapter/$file")
    done
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

# The framework adapters (points 77–79). Packed with pnpm, which writes the
# version of `@casoon/opengrid` into the peer range where the workspace has
# `workspace:^` — npm would ship the protocol as it is, and no one could
# install it. pnpm resolves that version from the installed workspace, so the
# workspace is installed first (a no-op when it already is; on a fresh clone
# or a CI runner it is the step that makes packing possible).
# stdout only is quiet: an install that fails says why on stderr.
pnpm install --frozen-lockfile >/dev/null
for adapter in "${adapters[@]}"; do
    adapter_dest="$root/target/npm-package-$adapter"
    rm -rf "$adapter_dest"
    mkdir -p "$adapter_dest"
    (cd "$root/packages/opengrid-$adapter" && pnpm pack --pack-destination "$adapter_dest" >/dev/null)
    adapter_tarball="$(cd "$adapter_dest" && ls *.tgz)"
    tar -xzf "$adapter_dest/$adapter_tarball" -C "$adapter_dest"

    echo
    echo "packed:   $adapter_dest/$adapter_tarball"
    echo "unpacked: $adapter_dest/package"
    echo
    echo "contents:"
    tar -tzf "$adapter_dest/$adapter_tarball" | sort
done
