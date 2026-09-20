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
for file in LICENSE-MIT LICENSE-APACHE README.md; do
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
