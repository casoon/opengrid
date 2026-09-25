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
react="$root/packages/opengrid-react"
for file in LICENSE-MIT LICENSE-APACHE; do
    cp "$root/$file" "$react/$file"
    staged+=("$react/$file")
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

# The React adapter (point 77). Packed with pnpm, which writes the version of
# `@casoon/opengrid` into the peer range where the workspace has `workspace:^`
# — npm would ship the protocol as it is, and no one could install it.
react_dest="$root/target/npm-package-react"
rm -rf "$react_dest"
mkdir -p "$react_dest"
(cd "$react" && pnpm pack --pack-destination "$react_dest" >/dev/null)
react_tarball="$(cd "$react_dest" && ls *.tgz)"
tar -xzf "$react_dest/$react_tarball" -C "$react_dest"

echo
echo "packed:   $react_dest/$react_tarball"
echo "unpacked: $react_dest/package"
echo
echo "contents:"
tar -tzf "$react_dest/$react_tarball" | sort
