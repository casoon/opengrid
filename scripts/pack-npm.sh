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

# The module the package ships. Built here so `files` has something to find.
just wasm-build-components

# Staged, not authored: the licences and the readme live at the repository root
# and are copied in for the tarball. Removed again below so the working tree
# keeps exactly one copy of each.
staged=()
for file in LICENSE-MIT LICENSE-APACHE README.md CHANGELOG.md; do
    cp "$root/$file" "$pkg/$file"
    staged+=("$pkg/$file")
done
# The framework adapters (points 77, 78) ship the licences too.
adapters=(react vue)
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

# The framework adapters (points 77, 78). Packed with pnpm, which writes the
# version of `@casoon/opengrid` into the peer range where the workspace has
# `workspace:^` — npm would ship the protocol as it is, and no one could
# install it. pnpm resolves that version from the installed workspace, so the
# workspace is installed first (a no-op when it already is; on a fresh clone
# or a CI runner it is the step that makes packing possible).
pnpm install --frozen-lockfile --silent
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
