# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Entries for versions prior to `2.0.0` were reconstructed from the git history
and may be incomplete.

## [Unreleased]

_No unreleased changes yet._

## [2.0.0] - 2026-07-21

A maintenance and hardening release. The public API was simplified, the
`download` module was reorganized, and several correctness bugs were fixed.
**The API changes are breaking** (hence the major bump).

### Changed (breaking)

- `Download::download` now takes `&self` instead of `self`, so a single
  `Download` instance can be reused across multiple downloads.
- Removed the unused lifetime parameter `'a` and `Cow<'a, str>` everywhere
  across `Download`, `DownloadOptions`, `DownloadReport`, and the `utils`
  helpers. String fields now use owned `String` values.
- `utils::is_file`, `utils::is_dir`, and `utils::get_file_size` are now
  `async fn`s backed by `tokio::fs::metadata`. They no longer block the
  tokio runtime, and they no longer return `AnyResult` (the soft-check
  semantics always returned `false`/`0` on missing paths anyway — the
  signatures now reflect that).
- `utils::gen_file_name` returns `String` directly (no `Result`) and formats
  the timestamp with `%Y%m%dT%H%M%S` so the generated name is filename-safe
  on Windows.
- Rewrote the HEAD request retry loop in `Download::download`: it now retries
  at most `MAX_HEAD_REQUEST_RETRIES` (5) times with a fixed delay instead of
  relying on the broken status-code comparison.
- The progress bar is only constructed when `show_progress == true`; it is
  also properly finished on completion.
- CLI `--version` is now driven by `env!("CARGO_PKG_VERSION")` so it stays
  in sync with `Cargo.toml`.
- Enabled `clippy::pedantic` for the crate.

### Changed

- Reorganized the `download` module from a single 781-line `download.rs`
  into a module directory with focused files:
  - `download/mod.rs` — `Download` type and the `download()` core
  - `download/options.rs` — `DownloadOptions` and its builder
  - `download/report.rs` — `DownloadReport` and `DownloadStatus`
  - `download/client.rs` — extracted HTTP client construction helper
- Extracted `build_client()` into `download::client`, deduplicating the
  proxy/no-proxy client setup; timeouts now live in one place.
- Extracted `build_progress_bar()` helper out of `Download::download` so the
  progress-bar styling is testable and the download loop is shorter.
- Relaxed direct dependency constraints from pinned caret (e.g. `^4.6`) to
  major-only (e.g. `4`) and refreshed `Cargo.lock` via `cargo update`,
  pulling in newer patch/minor releases across the dependency tree.

### Added

- HTTP client request timeout (60s) and connect timeout (10s) to avoid
  hangs on slow or unresponsive servers.
- CLI `-u`/`--url` flag as an alternative to the positional URL argument
  (the flag was advertised in `README.md` but not previously implemented).
- `rust-version = "1.85"` MSRV declaration in `Cargo.toml`.
- `clippy.toml` with `msrv = "1.85"`.
- `rust-toolchain.toml` pinning the stable channel with `rustfmt` and
  `clippy` components.
- Dedicated `ci.yml` workflow: `rustfmt`, `clippy -D warnings`, and a test
  matrix of `{ubuntu, macos, windows} × {stable, 1.85 (MSRV)}`.
- `LICENSE` file (MIT) — the MIT license was declared in `Cargo.toml` but
  the text was missing from the repo.
- `#[must_use]` on `Download::new`, `DownloadOptions::new`, and
  `DownloadReport::new` so callers don't accidentally drop instances.
- `docs/` article set (in Chinese) for community promotion:
  - `docs/01-introduction.md` — project overview
  - `docs/02-getting-started.md` — 5-minute quickstart (CLI + library)
  - `docs/03-deep-dive.md` — source-level technical walkthrough
  - `docs/README.md` — article index + publishing guide

### Fixed

- HEAD request retry loop: the retry counter constant was mistakenly
  compared against the HTTP status code (`status >= MAX_HEAD_REQUEST_RETRIES`
  with the constant equal to `5`), which caused the loop to break on the
  first iteration and the retry mechanism to never fire.
- `utils::gen_file_name` produced timestamps containing `:`, which is an
  illegal filename character on Windows.
- CLI `--progress` argument combined `default_value("true")` with
  `ArgAction::SetTrue`, which conflicted and produced surprising defaults.
- Removed a misleading `use -u <url>` hint from the CLI error message (the
  flag did not exist at the time).
- `DownloadReport::report` now serializes via the derived `Serialize` impl
  instead of hand-building a `json!({...})` object, so the wire format
  cannot drift away from the struct definition.
- Running the CLI with no arguments now prints full `--help` instead of a
  one-line "URL is required" error, matching the `git`/`cargo` convention.

### Documentation

- Rewrote `AGENTS.md` to match the 2.0 codebase: new module layout, the
  drop of `Cow<'a, str>`, `clippy::pedantic` policy, MSRV (1.85), the
  full `cargo fmt/clippy/test/doc` command set, and a PR-readiness
  checklist.
- Corrected `skills/SKILL.md`: `--progress` default is `false` (not `true`),
  matching the actual CLI.
- Updated `README.md`: JSON output example now reflects all `DownloadReport`
  fields (`range_from`, timestamps, `msg`); added MSRV note, timeouts to the
  feature list, and links to `CHANGELOG.md` / `AGENTS.md`.

## [1.0.1] - 2026-05-07

### Added

- Automated release workflow via `cargo-dist`
  (`.github/workflows/release.yml`) producing shell and PowerShell
  installers and cross-platform binaries.
- `SKILL.md` contributor documentation.

## [1.0.0] - 2026-01-28

### Added

- `siwi-download` CLI binary (`-o`, `-f`, `-P`, `-p`, `-v`, `-j` options).
- JSON output for download reports via `--json` / `-j`.
- Serde `Serialize`/`Deserialize` for `DownloadReport` and `DownloadStatus`.

### Changed

- Upgraded to Rust edition 2024.
- Upgraded `reqwest` to `0.13` and `indicatif` to `0.18`.

## [0.3.0] - 2025-01-09

### Added

- Further refinements to the core download loop.

### Changed

- Upgraded `reqwest` to `0.12`.

### Fixed

- File size calculation when resuming an interrupted download.

## [0.2.1] - 2021-02-24

### Added

- `DownloadReport` and related download result metadata.
- `utils` module with file/directory helpers.
- First GitHub Actions workflow (`rust.yml`).
- Additional documentation.

## [0.2.0] - 2021-02-20

### Added

- Initial public release of `siwi-download`.
- Core async download API built on tokio + reqwest with breakpoint
  continuation support.
- `DownloadReport` with `origin_file_name`, `resp_status`, and `msg` fields.
- Library `examples/` and unit tests.
- `Cow<'a, str>` used for lifetime-efficient string handling in the API.

<!-- Link references -->
[Unreleased]: https://github.com/rs-videos/siwi-download/compare/v2.0.0...HEAD
[2.0.0]: https://github.com/rs-videos/siwi-download/compare/v1.0.1...v2.0.0
[1.0.1]: https://github.com/rs-videos/siwi-download/compare/v1.0.0...v1.0.1
[1.0.0]: https://github.com/rs-videos/siwi-download/compare/v0.3.0...v1.0.0
[0.3.0]: https://github.com/rs-videos/siwi-download/compare/v0.2.1...v0.3.0
[0.2.1]: https://github.com/rs-videos/siwi-download/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/rs-videos/siwi-download/releases/tag/v0.2.0
