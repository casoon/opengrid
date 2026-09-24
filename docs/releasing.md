# Releasing opengrid

What a release of `@casoon/opengrid` consists of, in the order it happens.

Most of this is one command. The steps that are **not** a command are marked 👤 — they
need a person, a real browser and, for the accessibility run, real assistive technology.
A release that skips them is not a release; it is a publish.

## What 0.x promises

The public API is frozen (the element names, their attributes, their events, their
`::part` names, their custom properties, their text keys and the eight exported
functions) and `docs/api.md` is its written form. Tests keep the two honest with each
other: a frozen name missing from the documentation fails, and so does a documented
part the element does not write.

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

These scale with the version. Demanding the full matrix of a `0.1.0` would mean
demanding a QA lab of a project that has one developer, and the honest outcome of that
is not a careful release — it is no release, or a quiet exception. So the bar is
staged, and what was **not** tested is named in the release notes either way.

#### For any `0.x`

- [ ] **One pairing, in depth.** All twenty-eight scenarios of
      `plan/spezifikation/15-sr-testprotokoll.md` on a screen reader you actually have:
      S1–S18 cover the elements as such, S19–S28 the configurable views (grouping and
      its change of role, the column menu, the search combobox, facets, chips, the
      language of mixed names). Several depend on `:focus` and cannot be driven from
      the console — S9, S10, and every scenario that opens a menu — they need a real
      keyboard.
- [ ] **Name the gaps.** Every pairing you did not test goes into the release notes by
      name. "Tested with VoiceOver on Safari; NVDA, JAWS and TalkBack untested" is a
      useful sentence. Silence is not.
- [ ] **One browser beyond Chromium**, by hand. The e2e suite runs Chromium only, so
      everything else is either checked by a person or unknown.
- [ ] **The screenshot baselines.** `tests/e2e/__screenshots__` is committed, once per
      platform. If a visible control changed, regenerate the darwin ones with
      `pnpm run e2e:update` and the Linux ones by running the CI workflow by hand
      (`gh workflow run ci.yml`; its `baselines` job uploads them as an artifact), and
      **look at the diff** of both — a baseline accepted without looking is a test that
      has stopped testing.

#### Additionally for `1.0`

- [ ] **The full pairing matrix**: NVDA with Firefox and with Chrome, JAWS with Chrome,
      VoiceOver with Safari on macOS and on iOS, TalkBack with Chrome on Android
      (`plan/spezifikation/09-accessibility.md`). At 1.0 an untested pairing stops being
      a documented gap and becomes a blocker.
- [ ] **The full browser matrix**: Chrome, Edge, Firefox, Safari.

#### What is automated, so you do not have to listen for it

`tests/e2e/announcements.spec.js` records the status line's **successive** states and
pins the sequence, which catches the three mechanical failures — a thing said twice, a
thing never said, a thing said too early and swallowed by the next result. It found one
the first time it ran: turning a page announced nothing, because the row count is the
same on every page.

What it cannot do is hear. Whether the live region interrupts or queues, whether the
wording is comprehensible, whether the whole thing is usable rather than merely
conformant — that is what the run is for, and why it is a judgement rather than an
inventory.

## The release

- [ ] `just set-version 0.1.0` — sets the version in all three places that print
      one: the Cargo workspace, the npm package and the project page's header
      badge. They drifted once, and the page shows its version publicly, so this
      is one command rather than three files to remember. All three read `0.0.0`
      while nothing is released.
- [ ] Move the `Unreleased` section of [CHANGELOG.md](../CHANGELOG.md) under the new
      version with today's date, and say plainly what breaks if this is a minor
      bump — plus which screen-reader pairings were tested and which were not.
      The 0.1.0 content is already written; what is missing from it is the
      sentence naming the pairings, which only the run can supply.
- [ ] `just package` — builds the module, stages the licences and the readme, packs and
      unpacks the tarball to `target/npm-package/`
- [ ] Read `target/npm-package/package/` and check that it holds what it should, and
      nothing more. CI does this on every push (`publish-dry-run`), but read it anyway
      before the one push that is real.
- [ ] `cd target/npm-package/package && npm publish` — publish the **packed** directory,
      not `packages/opengrid`, so what is published is what was tested
- [ ] Tag the commit, and push the tag. The remote is
      `https://github.com/casoon/opengrid.git`; the repository has to exist there
      first, and `package.json` already points `repository`/`homepage`/`bugs` at it.

## After

- [ ] Install the published package into an empty project and load a page from it. The
      packaged test proves the tarball is complete; only an install proves the registry
      has it.

## Not part of a release

A CDN build, a documentation site and framework adapters. None of them exist, and
inventing one during a release is how a release goes wrong.
