#!/usr/bin/env bash
# Publishes a release: the npm tarballs that CI built for the release commit,
# then the tag. `@casoon/opengrid` — the React, Vue and Svelte components are
# its subpaths (#27) — and `@casoon/opengrid-mcp` (#140), with one version.
#
# `--mcp` publishes `@casoon/opengrid-mcp` alone, under its own tag
# `opengrid-mcp-vX.Y.Z`: for a version whose `@casoon/opengrid` is on npm
# already (0.13.0, its first release).
#
# What is published is what CI checked: the `publish-dry-run` job packs with the
# pinned toolchain and uploads the tarballs as the artifact `npm-packages`, and
# this script downloads exactly that artifact for the commit it releases — never
# a local build. See docs/releasing.md → The release.
#
# Usage: pnpm release            (asks before anything is published)
#        pnpm release --dry-run  (every check, `npm publish --dry-run`, no tag)
#        pnpm release --mcp [--dry-run]
set -euo pipefail

cd "$(dirname "$0")/.."

dry_run=false
mcp_only=false
for arg in "$@"; do
    case "$arg" in
        --dry-run) dry_run=true ;;
        --mcp) mcp_only=true ;;
        *) echo "usage: pnpm release [--mcp] [--dry-run]" >&2; exit 2 ;;
    esac
done

fail() { echo "release: $*" >&2; exit 1; }

version=$(node -p 'require("./packages/opengrid/package.json").version')
mcp_version=$(node -p 'require("./packages/opengrid-mcp/package.json").version')
[ "$mcp_version" = "$version" ] || fail "opengrid-mcp is $mcp_version, opengrid $version — run \`just set-version\`"
dest="target/release"

# The artifact holds the tarballs at its root: CI uploads `target/npm-package/*.tgz`.
# In this order: the MCP server depends on the grid of its version.
if $mcp_only; then
    tag="opengrid-mcp-v$version"
    packages=("casoon-opengrid-mcp-$version.tgz")
else
    tag="v$version"
    packages=("casoon-opengrid-$version.tgz" "casoon-opengrid-mcp-$version.tgz")
fi

# --- The commit -------------------------------------------------------------

[ "$version" != "0.0.0" ] || fail "the version is 0.0.0 — run \`just set-version <x.y.z>\` first"
[ "$(git branch --show-current)" = "main" ] || fail "not on main"
git fetch -q origin main --tags
commit=$(git rev-parse HEAD)
[ "$commit" = "$(git rev-parse origin/main)" ] || fail "main is not at origin/main — pull or push first"
[ -z "$(git status --porcelain --untracked-files=no)" ] || fail "the working tree has changes"
if git rev-parse -q --verify "refs/tags/$tag" >/dev/null || [ -n "$(git ls-remote --tags origin "refs/tags/$tag")" ]; then
    fail "$tag exists already"
fi
npm whoami >/dev/null 2>&1 || fail "not logged in to npm — run \`npm login\`"
if $mcp_only; then
    npm view "@casoon/opengrid@$version" version >/dev/null 2>&1 \
        || fail "@casoon/opengrid@$version is not on npm — release both, without --mcp"
fi

# --- The tarballs CI built for it --------------------------------------------

run=$(gh run list --workflow ci.yml --branch main --event push --commit "$commit" \
    --status success --limit 1 --json databaseId -q '.[0].databaseId')
[ -n "$run" ] || fail "no successful CI run on main for ${commit:0:7} yet — wait for it"

rm -rf "$dest"
gh run download "$run" -n npm-packages -D "$dest"

for file in "${packages[@]}"; do
    [ -f "$dest/$file" ] || fail "the artifact has no $file"
    manifest=$(tar -xzOf "$dest/$file" package/package.json)
    got=$(node -p 'JSON.parse(process.argv[1]).version' "$manifest")
    [ "$got" = "$version" ] || fail "$file says $got, not $version"
    ! grep -q '"workspace:' <<<"$manifest" || fail "$file still names a workspace: range"
done

echo
echo "  version  $version"
echo "  commit   ${commit:0:7} ($(git log -1 --format=%s "$commit"))"
echo "  CI run   $run"
for file in "${packages[@]}"; do
    echo "  publish  $file"
done
echo "  tag      $tag on ${commit:0:7}"
echo

# --- Publish -----------------------------------------------------------------

if $dry_run; then
    for file in "${packages[@]}"; do
        npm publish --dry-run "$dest/$file" >/dev/null 2>&1 || fail "npm publish --dry-run refused $file"
        echo "dry run: $file would publish"
    done
    echo "dry run: nothing was published, no tag was set."
    exit 0
fi

read -r -p "This cannot be undone. Type $version to publish: " answer
[ "$answer" = "$version" ] || fail "not confirmed — nothing was published"

for file in "${packages[@]}"; do
    name=$(node -p 'JSON.parse(process.argv[1]).name' "$(tar -xzOf "$dest/$file" package/package.json)")
    # A release interrupted halfway is finished by running it again.
    if npm view "$name@$version" version >/dev/null 2>&1; then
        echo "$name@$version is on npm already — skipped"
        continue
    fi
    npm publish "$dest/$file"
done

git tag "$tag" "$commit"
git push origin "$tag"
# The draft release, if there is one, belongs to this commit.
gh release edit "$tag" --target "$commit" >/dev/null 2>&1 || true

echo
echo "Published $version and pushed $tag."
echo "Next: publish the draft release on GitHub, and merge the docs that say it is on npm."
