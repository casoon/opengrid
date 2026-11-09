# Contributing

Work is tracked as [GitHub issues](https://github.com/casoon/opengrid/issues).
Each issue states what is decided, the tasks, "Done when" and "Not part of
this"; what it states as decided is binding. Open or pick an issue before
starting a larger change.

- One branch and one pull request per issue, with `Closes #N` in the body.
- Run `just check` (format, clippy with `-D warnings`, tests) before pushing;
  run what the issue's "Done when" names and say in the PR what you could not
  run. `just wasm-check` and `just e2e` cover the wasm build and the browser
  suite.
- A new check should fail when the thing it guards is broken: try it against a
  deliberate mutation.
- Non-browser crates must not depend on `web-sys` or `js-sys`.
- No new dependency without a reason stated in the PR.
- User-visible changes get an entry under `## [Unreleased]` in `CHANGELOG.md`
  ([Keep a Changelog](https://keepachangelog.com/en/1.1.0/)).

Security problems: see [SECURITY.md](SECURITY.md), not a public issue.
