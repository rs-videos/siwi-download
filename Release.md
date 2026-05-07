# Release Process for siwi-download

This document outlines the automated release process for siwi-download using cargo-dist.

## Overview

cargo-dist automates the entire release workflow:
- Cross-platform builds (macOS, Linux, Windows)
- Shell and PowerShell installer generation
- GitHub Releases creation
- Checksum generation

## Prerequisites

- GitHub CLI (`gh`) authenticated
- Write access to `rs-videos/siwi-download` repository
- Git remote properly configured

## Release Steps

### 1. Update Version

Update the version in `Cargo.toml`:

```toml
[package]
version = "X.Y.Z"
```

### 2. Commit and Tag

```bash
# Add and commit changes
git add .
git commit -m "Release vX.Y.Z"

# Create and push tag
git tag vX.Y.Z
git push origin main --tags
```

### 3. GitHub Actions Workflow

The release workflow (`.github/workflows/release.yml`) will automatically:

1. **Build** - Compiles binaries for all platforms:
   - macOS ARM64 (`aarch64-apple-darwin`)
   - macOS Intel (`x86_64-apple-darwin`)
   - Linux (`x86_64-unknown-linux-gnu`)
   - Windows (`x86_64-pc-windows-msvc`)

2. **Package** - Creates compressed archives with checksums:
   - `.tar.xz` for Unix-like systems
   - `.zip` for Windows

3. **Generate Installers**:
   - Shell installer (`siwi-download-installer.sh`) - Unix/Linux/macOS
   - PowerShell installer (`siwi-download-installer.ps1`) - Windows

4. **Publish** - Creates GitHub Release with all artifacts

## Installation Methods

After release, users can install using:

### Shell (Linux/macOS)

```bash
curl -LsSf https://github.com/rs-videos/siwi-download/releases/latest/download/siwi-download-installer.sh | sh
```

### PowerShell (Windows)

```powershell
irm https://github.com/rs-videos/siwi-download/releases/latest/download/siwi-download-installer.ps1 | iex
```

### Direct Download

Download pre-built binaries from the GitHub Releases page:
- https://github.com/rs-videos/siwi-download/releases

## Local Testing

Test the build locally before pushing:

```bash
# Plan the release (preview what will be built)
dist plan

# Build for current platform
dist build

# Build for specific target
dist build --target aarch64-apple-darwin
```

## Troubleshooting

### Build Failures

1. Ensure Rust toolchain is up to date:
   ```bash
   rustup update
   ```

2. Check for compilation errors:
   ```bash
   cargo build --release
   ```

### Release Not Triggered

Make sure the tag follows semver format:
- Valid: `v1.0.0`, `v0.1.0-beta.1`, `v2.3.4`
- Invalid: `v1`, `release-1.0`, `1.0.0`

### Installer Issues

If installers fail to generate:
1. Check GitHub Actions logs
2. Verify `dist-workspace.toml` configuration
3. Ensure all targets are properly specified
