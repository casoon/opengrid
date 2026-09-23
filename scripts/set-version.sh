#!/usr/bin/env bash
# Sets the release version in every place that prints one.
#
# They drifted once already: the npm package said 0.1.0 while the crates and the
# project page said 0.0.0, and the page prints its version in the header, so the
# disagreement was public. Three files, one command, no hunting.
#
# Usage: bash scripts/set-version.sh 0.1.0
set -euo pipefail

cd "$(dirname "$0")/.."
version="${1:?usage: set-version.sh <x.y.z>}"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] || {
    echo "not a semver version: $version" >&2
    exit 1
}

# Cargo workspace: the first `version = "..."` under [workspace.package].
perl -0pi -e "s/(\[workspace\.package\]\nversion = \")[^\"]+(\")/\${1}$version\${2}/" Cargo.toml
# npm package.
perl -0pi -e "s/(\"version\": \")[^\"]+(\")/\${1}$version\${2}/" packages/opengrid/package.json
# The project page's header badge.
perl -0pi -e "s/(version: ')[^']+(')/\${1}$version\${2}/" site/astro.config.mjs

echo "set to $version:"
grep -m1 -A1 '^\[workspace.package\]' Cargo.toml | tail -1
grep -m1 '"version"' packages/opengrid/package.json
grep -m1 "version: '" site/astro.config.mjs

# Cargo.lock carries the workspace members' versions too.
cargo metadata --format-version 1 >/dev/null 2>&1 || true
echo
echo "Now: update CHANGELOG.md (move Unreleased under [$version]) and run the"
echo "checklist in docs/releasing.md."
