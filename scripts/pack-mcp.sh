#!/usr/bin/env bash
# Packs `@casoon/opengrid-mcp` the way a release would (#140), next to
# `@casoon/opengrid`'s tarball in target/npm-package/.
#
# pnpm packs it, not npm: `@casoon/opengrid` is a `workspace:*` dependency, and
# only pnpm writes the version it stands for. The view must be built first
# (`just mcp-elements`), the tarball ships dist/view.html.
#
# Usage: bash scripts/pack-mcp.sh   (after `just mcp-elements`)
set -euo pipefail

cd "$(dirname "$0")/.."
root="$PWD"
pkg="$root/packages/opengrid-mcp"
dest="$root/target/npm-package"

[[ -f "$pkg/dist/view.html" ]] || {
    echo "no dist/view.html — run \`just mcp-elements\` first" >&2
    exit 1
}

# Staged, not authored, as in pack-npm.sh: the licences live at the root.
staged=()
for file in LICENSE-MIT LICENSE-APACHE; do
    cp "$root/$file" "$pkg/$file"
    staged+=("$pkg/$file")
done
cleanup() { rm -f "${staged[@]}"; }
trap cleanup EXIT

mkdir -p "$dest"
rm -f "$dest"/casoon-opengrid-mcp-*.tgz
(cd "$pkg" && pnpm pack --pack-destination "$dest" >/dev/null)
tarball="$(ls "$dest"/casoon-opengrid-mcp-*.tgz)"

manifest="$(tar -xzOf "$tarball" package/package.json)"
! grep -q '"workspace:' <<<"$manifest" || {
    echo "$tarball still names a workspace: range" >&2
    exit 1
}
for file in package/src/server.mjs package/dist/view.html package/LICENSE-MIT package/LICENSE-APACHE; do
    tar -tzf "$tarball" | grep -qx "$file" || {
        echo "$tarball has no $file" >&2
        exit 1
    }
done

echo "packed: $tarball"
