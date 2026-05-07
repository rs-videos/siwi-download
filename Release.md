# Release Process for siwi-download

This document outlines the steps to release a new version of siwi-download to Homebrew.

## Prerequisites

- Homebrew installed on macOS
- Access to `rs-videos/homebrew-tap` repository (write access)
- GitHub CLI (`gh`) authenticated

## Release Steps

### 1. Update Version in Cargo.toml

Update the `version` field in `Cargo.toml`:

```toml
[package]
version = "X.Y.Z"
```

### 2. Build Binaries for All Platforms

Build binaries for both Apple Silicon and Intel macOS:

```bash
# Build for Apple Silicon (aarch64-apple-darwin)
cargo build --release --target aarch64-apple-darwin

# Build for Intel (x86_64-apple-darwin)
cargo build --release --target x86_64-apple-darwin
```

### 3. Create Release Directory and Binaries

```bash
# Create release directory
mkdir -p release-build

# Copy binaries with versioned names
cp target/aarch64-apple-darwin/release/siwi-download release-build/siwi-download-vX.Y.Z-aarch64-apple-darwin
cp target/x86_64-apple-darwin/release/siwi-download release-build/siwi-download-vX.Y.Z-x86_64-apple-darwin
```

### 4. Create Tarballs

```bash
cd release-build

# Create tar.gz archives
tar -czvf siwi-download-vX.Y.Z-aarch64-apple-darwin.tar.gz siwi-download-vX.Y.Z-aarch64-apple-darwin
tar -czvf siwi-download-vX.Y.Z-x86_64-apple-darwin.tar.gz siwi-download-vX.Y.Z-x86_64-apple-darwin
```

### 5. Calculate SHA256 Checksums

```bash
# Generate checksums for both tarballs
sha256sum siwi-download-vX.Y.Z-aarch64-apple-darwin.tar.gz siwi-download-vX.Y.Z-x86_64-apple-darwin.tar.gz

# Example output:
# abc123...  siwi-download-vX.Y.Z-aarch64-apple-darwin.tar.gz
# def456...  siwi-download-vX.Y.Z-x86_64-apple-darwin.tar.gz
```

### 6. Create GitHub Release

Using GitHub CLI:

```bash
gh release create vX.Y.Z \
  --title "siwi-download vX.Y.Z" \
  --notes "Release notes here" \
  release-build/siwi-download-vX.Y.Z-aarch64-apple-darwin.tar.gz \
  release-build/siwi-download-vX.Y.Z-x86_64-apple-darwin.tar.gz
```

Or manually via GitHub web interface:
1. Go to https://github.com/rs-videos/siwi-download/releases/new
2. Tag: `vX.Y.Z`
3. Title: `siwi-download vX.Y.Z`
4. Upload both tar.gz files
5. Publish release

### 7. Update Homebrew Formula

Clone and update the homebrew-tap repository:

```bash
# Clone homebrew-tap repo
git clone git@github.com:rs-videos/homebrew-tap.git
cd homebrew-tap

# Edit the formula
vim Formula/siwi-download.rb
```

Update the following fields:

```ruby
class SiwiDownload < Formula
  desc "Downloader with pure HTTP implementation supporting breakpoint continuation"
  homepage "https://github.com/rs-videos/siwi-download"
  license "MIT"
  version "X.Y.Z"

  url "https://github.com/rs-videos/siwi-download/releases/download/vX.Y.Z/siwi-download-vX.Y.Z-aarch64-apple-darwin.tar.gz"
  sha256 "SHA256_HASH_FOR_aarch64"

  # Optional: Add x86_64 checksum if you want to support both architectures
  # on Intel Macs (bottles are preferred but this works)
  # sha256 "SHA256_HASH_FOR_x86_64"

  def install
    bin.install "siwi-download"
  end

  test do
    assert_match version, shell_output("#{bin}/siwi-download --version").strip
  end
end
```

### 8. Commit and Push Changes

```bash
# Stage changes
git add Formula/siwi-download.rb

# Commit with conventional format
git commit -m "siwi-download X.Y.Z"

# Push to remote
git push origin main
```

### 9. Verify Installation

```bash
# Test the new formula
brew uninstall siwi-download
brew install siwi-download

# Verify version
siwi-download --version
```

## Quick Reference

| Step | Command |
|------|---------|
| Build Apple Silicon | `cargo build --release --target aarch64-apple-darwin` |
| Build Intel | `cargo build --release --target x86_64-apple-darwin` |
| Create tarball | `tar -czvf siwi-download-vX.Y.Z-PLATFORM.tar.gz siwi-download-vX.Y.Z-PLATFORM` |
| Get SHA256 | `shasum -a 256 siwi-download-vX.Y.Z-PLATFORM.tar.gz` |
| Create release | `gh release create vX.Y.Z --title "siwi-download vX.Y.Z" <files>` |
| Push formula | `git add . && git commit -m "vX.Y.Z" && git push` |

## Troubleshooting

### Checksum Mismatch
If you get a checksum mismatch error after updating the formula:
1. Run `brew style --fix Formula/siwi-download.rb` to auto-fix style issues
2. Verify checksums match exactly what's in the release assets

### Binary Not Found
Ensure the binary name in the tarball matches exactly what `bin.install` expects:
- Tarball contents should be: `./siwi-download` (not in a subdirectory)

### Permission Denied
If pushing to homebrew-tap fails:
1. Verify SSH key has write access to `rs-videos/homebrew-tap`
2. Run `ssh-add -l` to check loaded keys
3. Ensure remote URL is correct: `git@github.com:rs-videos/homebrew-tap.git`
