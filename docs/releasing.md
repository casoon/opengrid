# Releasing opengrid

What a release of `@casoon/opengrid` consists of, in the order it happens.

Most of this is one command. The steps that are **not** a command are marked 👤 — they
need a person, a real browser and, for the accessibility run, real assistive technology.
A release that skips them is not a release; it is a publish.

## What 0.x promises

The public API is frozen (the element names, their attributes, their events, their
`::part` names and the four exported functions) and `docs/api.md` is its written form.
A test keeps that list honest.

While the version is **0.x**:

- **A minor bump (`0.1` → `0.2`) may break the API.** Read the changelog before upgrading.
- **A patch bump (`0.1.0` → `0.1.1`) may not.** It fixes behaviour, never renames anything.
- The npm package and the Rust crates carry the **same version** and are bumped together,
  even when only one of them changed. One number, one release.

1.0 is the point at which a minor bump stops being allowed to break anything. It is not
scheduled, and nothing below it should be read as a promise that it will be.

## Before the release

- [ ] `just check` — fmt, clippy with `-D warnings`, the whole test suite
- [ ] `just wasm-check` — every wasm-capable crate builds for `wasm32-unknown-unknown`
- [ ] `just e2e` — Playwright and axe-core, including `packaged.spec.js`, which loads the
      **packed** package rather than the repository
- [ ] `cargo check -p opengrid-web-components --no-default-features --features grid`, and
      the same with `--features pivot` — E25 keeps both as a way out, so both must build
- [ ] `just measure-modules` — if a number moved noticeably, the table in
      `plan/spezifikation/12-qualitaet.md` §Elementmodul und Modultrennung is stale, and
      E25 was decided on those numbers

### 👤 The parts no command covers

- [ ] **The screen-reader run.** The protocol is
      `plan/spezifikation/15-sr-testprotokoll.md`; the matrix is six pairings — NVDA with
      Firefox and with Chrome, JAWS with Chrome, VoiceOver with Safari on macOS and on
      iOS, TalkBack with Chrome on Android. A pairing that was not tested is a **gap in
      the release notes**, not a tick. Two scenarios (S9, focus while scrolling, and S10,
      the unfolding of a truncated cell) depend on `:focus` and cannot be driven from the
      console — they need a real keyboard.
- [ ] **The browser matrix.** Chrome, Edge, Firefox, Safari. The e2e suite runs Chromium
      only, so three of the four are only ever covered by hand.
- [ ] **The screenshot baselines.** `tests/e2e/__screenshots__` is committed. If a visible
      control changed, regenerate with `pnpm run e2e:update` and **look at the diff** — a
      baseline accepted without looking is a test that has stopped testing.

## The release

- [ ] Bump the version in `Cargo.toml` (`[workspace.package]`) and in
      `packages/opengrid/package.json` to the same number
- [ ] Write the changelog entry, and say plainly what breaks if this is a minor bump
- [ ] `just package` — builds the module, stages the licences and the readme, packs and
      unpacks the tarball to `target/npm-package/`
- [ ] Read `target/npm-package/package/` and check that it holds what it should, and
      nothing more. CI does this on every push (`publish-dry-run`), but read it anyway
      before the one push that is real.
- [ ] `cd target/npm-package/package && npm publish` — publish the **packed** directory,
      not `packages/opengrid`, so what is published is what was tested
- [ ] Tag the commit, and push the tag

## After

- [ ] Install the published package into an empty project and load a page from it. The
      packaged test proves the tarball is complete; only an install proves the registry
      has it.

## Not part of a release

A CDN build, a documentation site and framework adapters. None of them exist, and
inventing one during a release is how a release goes wrong.
