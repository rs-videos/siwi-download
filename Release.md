# Release Process for siwi-download

This document outlines how to cut a release of siwi-download. There are two
parts that must both happen:

1. **Publish the library to crates.io** — manual, run locally.
2. **Build and publish binaries via cargo-dist** — automated, triggered by
   pushing a version tag.

> The rest of this file is the source of truth for cutting a release. If
> anything here disagrees with `release.yml`, the workflow wins.

## Prerequisites

- Push access to `rs-videos/siwi-download`
- A crates.io account with an API token (`cargo login`)
- Rust 1.85+ (the MSRV), stable toolchain
- `cargo-dist` installed locally only if you want to preview the plan
  (`cargo install cargo-dist --version 0.31.0`)

## Pre-release Checklist

Run **all** of these locally before tagging. CI runs the same gates, but
catching failures here saves a tag-repush cycle.

```bash
# 1. All quality gates green
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo doc --no-deps            # must be warning-free

# 2. Version is the one you intend to release
grep '^version' Cargo.toml     # e.g. version = "2.0.0"

# 3. CHANGELOG.md has the version section with a real date
#    and [Unreleased] is empty

# 4. Working tree clean, main and develop in sync with origin
git status
git fetch --all
git log --oneline origin/main..main   # should be empty
git log --oneline main..origin/main   # should be empty

# 5. Dry-run the crates.io publish (catches missing metadata, etc.)
cargo publish --dry-run --registry crates-io
```

## Step 1 — Publish to crates.io

The library crate itself is published to crates.io. This is **not** done by
cargo-dist — cargo-dist only produces GitHub Release binaries.

```bash
# Login once if you haven't already
cargo login

# Publish (use --registry crates-io if your global config points at a mirror)
cargo publish --registry crates-io
```

Verify at <https://crates.io/crates/siwi-download>.

## Step 2 — Tag and Push (triggers cargo-dist)

```bash
# Tag must be v<version> matching Cargo.toml
git tag v2.0.0
git push origin v2.0.0
```

Pushing the tag triggers `.github/workflows/release.yml`, which will:

1. Run `cargo-dist` to build binaries for all configured targets:
   - `aarch64-apple-darwin` (macOS ARM)
   - `x86_64-apple-darwin` (macOS Intel)
   - `aarch64-unknown-linux-gnu` (Linux ARM)
   - `x86_64-unknown-linux-gnu` (Linux x86_64)
   - `x86_64-pc-windows-msvc` (Windows)
2. Produce `.tar.xz` (Unix) / `.zip` (Windows) archives with checksums.
3. Generate installers:
   - `siwi-download-installer.sh` (shell, macOS/Linux)
   - `siwi-download-installer.ps1` (PowerShell, Windows)
4. Create the GitHub Release with auto-generated notes (derived from
   `CHANGELOG.md`).

Watch the run at
<https://github.com/rs-videos/siwi-download/actions/workflows/release.yml>.
Typical duration: 10–20 minutes.

## Step 3 — Verify

After the Release workflow succeeds:

- [ ] GitHub Release exists: <https://github.com/rs-videos/siwi-download/releases>
- [ ] All 5 platform archives are attached
- [ ] Both installer scripts are attached
- [ ] `CHANGELOG.md` notes were picked up as the release body
- [ ] The install one-liners work:

```bash
# Shell
curl -LsSf https://github.com/rs-videos/siwi-download/releases/latest/download/siwi-download-installer.sh | sh

# PowerShell
irm https://github.com/rs-videos/siwi-download/releases/latest/download/siwi-download-installer.ps1 | iex
```

- [ ] `siwi-download --version` reports the new version
- [ ] crates.io page shows the new version as "newest"

## Post-release Housekeeping

- Update the `[Unreleased]` section in `CHANGELOG.md` back to the placeholder
  if you moved entries out of it for this release.
- If this was a major/minor release (not patch), consider publishing an
  article in `docs/` or social posts.

## Local Testing (optional, before tagging)

Preview what cargo-dist would build without creating a release:

```bash
# Show what would be built
dist plan

# Build for the current host only
dist build
```

Requires `cargo install cargo-dist --version 0.31.0`.

## Troubleshooting

### Tag pushed but no release

- Check the tag matches `**[0-9]+.[0-9]+.[0-9]+*` (e.g. `v2.0.0`).
- Check the Release workflow's status page for failures.
- A common cause: the version in `Cargo.toml` doesn't match the tag.

### crates.io publish fails with "already exists"

You can't republish a version. Bump the patch and republish.

### Installer downloads 404

The installer filenames include the version. Make sure you're pointing at
`releases/latest/download/...` and not a specific version, unless that's
intended.

## What CI Does vs. What You Do

| Step                              | Who/What              |
|-----------------------------------|------------------------|
| Run tests/lint/fmt                | `ci.yml` (every push)  |
| Publish crate to crates.io        | **You** (manual)       |
| Build cross-platform binaries     | `release.yml` (on tag) |
| Create GitHub Release             | `release.yml` (on tag) |
| Edit release notes                | Optional, you          |
