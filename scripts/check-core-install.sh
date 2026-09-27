#!/usr/bin/env bash
# Installs the **packed** package the way a page without a framework would
# (#27): `npm install` of the tarball from `just package` into a scratch
# project that has no React, Vue or Svelte. The adapters' frameworks are
# optional peers, so the install must succeed and must not pull any of them
# in — and the core must import without them.
#
# Usage: bash scripts/check-core-install.sh   (after `just package`)
set -euo pipefail

cd "$(dirname "$0")/.."
root="$PWD"
tarball="$(ls "$root"/target/npm-package/casoon-opengrid-*.tgz 2>/dev/null | head -1)"
work="$root/target/core-install"

[[ -n "$tarball" ]] || {
    echo "no packed tarball in target/npm-package — run \`just package\` first" >&2
    exit 1
}

rm -rf "$work"
mkdir -p "$work"
echo '{ "name": "core-install-check", "private": true, "type": "module" }' > "$work/package.json"
# Offline: the package has no dependencies, so nothing needs the registry. An
# install that wanted a framework would fail here instead of fetching it.
(cd "$work" && npm install "$tarball" --offline --no-audit --no-fund --ignore-scripts >/dev/null)

for framework in react vue svelte @types/react; do
    if [[ -e "$work/node_modules/$framework" ]]; then
        echo "a core-only install pulled in $framework — its peer is not optional" >&2
        exit 1
    fi
done

# Importing the core touches no DOM and needs no framework.
(cd "$work" && node --input-type=module -e 'const m = await import("@casoon/opengrid"); if (typeof m.connect !== "function") throw new Error("connect missing");')
echo "core-only install: ok — no framework installed, the core imports ($tarball)"
