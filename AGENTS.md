# AGENTS.md - siwi-download

Guidelines for AI agents (and human contributors) working on this codebase.

## Project at a Glance

- **What**: async file downloader with breakpoint continuation, built on Tokio + Reqwest.
- **Version**: 2.0.0 (see `CHANGELOG.md`).
- **Edition**: Rust 2024.
- **MSRV**: `1.85` (declared as `rust-version` in `Cargo.toml`).
- **Toolchain**: `stable`, pinned via `rust-toolchain.toml`.

## Build / Lint / Test Commands

CI (`.github/workflows/ci.yml`) runs all of the following with `-D warnings`;
the same should pass locally before pushing.

```bash
# Build (library + binary + examples + tests)
cargo build --all-targets --verbose

# Run all tests
cargo test --all-targets --verbose

# Run a single test by name
cargo test test_name -- --nocapture

# Build & run examples
cargo run --example cli
cargo run --example download

# Install the CLI locally
cargo install --path .

# Format
cargo fmt              # apply
cargo fmt --check      # CI gate (does not write)

# Lint (clippy::pedantic is enabled in src/lib.rs)
cargo clippy --all-targets -- -D warnings

# Build docs (must be warning-free)
cargo doc --no-deps
```

## Module Layout

```
src/
├── lib.rs              crate root; enables clippy::pedantic, re-exports modules
├── main.rs             CLI binary (clap)
├── error.rs            AnyError / AnyResult type aliases (anyhow)
├── utils.rs            URL parsing, async fs helpers, timestamp formatting
└── download/
    ├── mod.rs          Download type + download() core + HTTP status constants
    ├── options.rs      DownloadOptions + builder
    ├── report.rs       DownloadReport + DownloadStatus
    └── client.rs       build_client() helper (shared timeouts/proxy policy)

examples/
├── cli.rs              CLI usage example
└── download.rs         Library API usage example

.github/workflows/
├── ci.yml              fmt + clippy + test matrix (ubuntu/macos/windows × stable/1.85)
└── release.yml         cargo-dist release automation (do not hand-edit)
```

## Code Style

### General

- Modern async/await; no blocking I/O inside async fns.
- Builder pattern for configuration (`DownloadOptions`).
- Use `?` for error propagation; reserve `match` for genuinely branching logic.
- Keep functions focused. `Download::download` is the one intentional exception
  (carries `#[allow(clippy::too_many_lines)`); prefer extracting helpers
  (see `build_client`, `build_progress_bar`) over growing it further.

### Formatting (`rustfmt.toml`)

- Edition 2024, 2-space indent.

### Naming

- **snake_case** — functions, variables, modules.
- **PascalCase** — structs, enums, traits.
- **CamelCase** — enum variants (`DownloadStatus::Complete`).
- **SCREAMING_SNAKE_CASE** — file-private constants (e.g. `HTTP_OK`).

### Imports

Group and order: external crates → `std` → `crate`. Example from `download/mod.rs`:

```rust
use crate::{
  error::AnyResult,
  utils::{create_dir_all, get_file_name_from_url, get_file_size, is_dir},
};
use indicatif::{ProgressBar, ProgressState, ProgressStyle};
use reqwest::header::{HeaderMap, HeaderValue, RANGE};
use reqwest::header::CONTENT_LENGTH;
use std::fmt::Write;
use std::path::Path;
use tokio::{
  fs,
  io::AsyncWriteExt,
  time::{Duration, sleep},
};
use tracing::{info, warn};
```

### Error Handling

- `error::AnyError = anyhow::Error`, `error::AnyResult<T> = anyhow::Result<T>`.
- Propagate with `?`. For user-facing error paths consider
  `.with_context(|| ...)` to attach a readable message.
- Log with `tracing`: `info!`/`warn!`/`error!`, with structured key/value
  fields (see the HEAD retry loop for an example).

### Async Patterns

- `#[tokio::main]` for binaries.
- Always use `tokio::fs::*` inside async contexts — **never** `std::fs::*`.
  The `utils::is_file` / `is_dir` / `get_file_size` helpers are async for this
  reason; use them rather than re-implementing.
- Retry loops: bounded counter + fixed/exponential delay (see the HEAD request
  loop in `download/mod.rs`).

### Builder Pattern (2.0 API)

The 2.0 API dropped the earlier `Cow<'a, str>` in favor of owned `String`.
Builders take `impl Into<String>`:

```rust
impl DownloadOptions {
  pub fn set_file_name<S: Into<String>>(&mut self, file_name: S) -> &mut Self {
    self.maybe_file_name = Some(file_name.into());
    self
  }
}
```

- `Download::download` takes `&self` so one instance can be reused.
- `utils::is_file` / `is_dir` / `get_file_size` return `bool` / `u64`
  directly (they are soft checks; the old `AnyResult` wrapper was discarded).

### Documentation

- `///` doc comments on all public items; `cargo doc --no-deps` must be
  warning-free (intra-doc links resolve).
- `clippy::pedantic` is enabled crate-wide; noisy lints are selectively
  `#![allow]`-ed at the top of `src/lib.rs`. Prefer fixing a warning over
  silencing it.

### Testing

- `#[cfg(test)] mod tests` co-located with the code.
- Async helpers → `#[tokio::test]`.
- Tests return `AnyResult<()>` when they perform fallible setup.
- Descriptive names: `test_get_file_name_from_url_basic`, not `test1`.

## Key Dependencies

| Crate            | Purpose                                          |
|------------------|--------------------------------------------------|
| `tokio`          | async runtime (fs, macros, rt-multi-thread, io-util) |
| `reqwest`        | HTTP client (rustls, json; default-features off) |
| `anyhow`         | error handling                                   |
| `tracing` (+sub) | structured logging                               |
| `chrono`         | timestamps with serde support                    |
| `indicatif`      | progress bars                                    |
| `clap`           | CLI parsing (derive)                             |
| `serde`/`serde_json` | serialization of `DownloadReport`            |

## Release Process

- Versions: SemVer, recorded in `CHANGELOG.md` (Keep a Changelog format).
- Pushing a `vX.Y.Z` tag triggers `release.yml` (cargo-dist) which builds
  cross-platform binaries + shell/PowerShell installers and creates the
  GitHub Release.
- See `Release.md` for the manual pre-release checklist.

## Conventions Checklist (before opening a PR)

- [ ] `cargo fmt --check` clean
- [ ] `cargo clippy --all-targets -- -D warnings` clean
- [ ] `cargo test --all-targets` passes
- [ ] `cargo doc --no-deps` warning-free
- [ ] Public API changes reflected in `CHANGELOG.md` `[Unreleased]`
- [ ] MSRV (`1.85`) still builds if you touched deps
