# SKILL.md - siwi-download

This document provides specialized knowledge and guidelines for AI assistants working on the siwi-download codebase.

## Project Overview

**siwi-download** is a Rust-based file downloader library and CLI tool built on `tokio` and `reqwest` with breakpoint continuation (resume) support.

### Core Features
- 🚀 Async HTTP downloads using `reqwest` with custom redirect handling
- 📊 Progress bar visualization via `indicatif`
- 🔄 Resume support via HTTP Range headers (206 Partial Content)
- 🌐 Proxy support (HTTP/HTTPS)
- 📁 Customizable output paths and filenames
- 📄 JSON output for programmatic consumption
- 🔧 Both CLI binary and library API

## Architecture

### Module Structure

```
src/
├── lib.rs      - Library root, exports public API
├── main.rs     - CLI binary entry point
├── error.rs    - Error types (AnyError, AnyResult aliases)
├── download.rs - Core download logic (Download struct, options, report)
└── utils.rs    - Utility functions (URL parsing, filename extraction)
```

### Key Types

| Type | Purpose |
|------|---------|
| `Download` | Main struct for performing downloads |
| `DownloadOptions` | Builder pattern for download configuration |
| `DownloadReport` | Result structure with download metadata |
| `DownloadStatus` | Enum for download state (Create, Append, etc.) |

## Build Commands

```bash
# Build project
cargo build --verbose

# Run all tests
cargo test --verbose

# Run specific test
cargo test test_name -- --nocapture

# Run examples
cargo run --example cli <url>
cargo run --example download

# Install CLI tool
cargo install siwi-download

# Format code
cargo fmt

# Check formatting
cargo fmt --check

# Lint with clippy
cargo clippy
```

## Code Style Guidelines

### General Principles
- Write clear, idiomatic Rust code using modern async/await patterns
- Prefer builder patterns for configuration objects
- Use `?` operator for error propagation instead of match blocks
- Keep functions focused and under ~50 lines when possible

### Formatting (rustfmt.toml)
- Edition: 2024
- Tab spaces: 2

### Naming Conventions

| Pattern | Usage |
|---------|-------|
| **snake_case** | Functions and variables: `get_file_name_from_url()` |
| **PascalCase** | Types, traits, structs: `DownloadOptions`, `DownloadReport` |
| **CamelCase** | Enum variants: `DownloadStatus::Create`, `DownloadStatus::Append` |
| **underscore prefix** | Private helper functions: `fn _internal_helper()` |

### Import Organization

```rust
// External crates first, then std, then crate
use crate::{
  error::AnyResult,
  utils::{get_file_name_from_url, get_file_size},
};
use chrono::{DateTime, Utc};
use reqwest::header::CONTENT_LENGTH;
use tokio::{fs, io::AsyncWriteExt};
```

## Error Handling

### Pattern Used

```rust
// error.rs
pub type AnyError = anyhow::Error;
pub type AnyResult<T> = anyhow::Result<T, AnyError>;
```

- Use `anyhow` for application errors
- Propagate errors with `?` operator
- Use `tracing` for logging: `error!("message {:?}", err)`

### Common Error Patterns
- Network failures via `reqwest` (automatically wrapped in anyhow)
- File system errors via `tokio::fs`
- Header parsing errors

## Async Patterns

- Use `tokio` runtime with `#[tokio::main]` for binaries
- Prefer `async/.await` over blocking I/O
- Use `tokio::fs` for file operations
- Retry logic with exponential backoff for network operations (see `download.rs`)

## Builder Pattern

Configuration uses builder pattern with setter methods:

```rust
impl<'a> DownloadOptions<'a> {
  pub fn set_file_name<S: Into<Cow<'a, str>>>(&mut self, file_name: S) -> &mut Self {
    self.maybe_file_name = Some(file_name.into());
    self
  }
}
```

- Use `Default` trait for default initialization
- Use `Cow<'a, str>` for lifetime-efficient string handling

## Key Dependencies

| Crate | Purpose |
|-------|---------|
| `tokio` | Async runtime (features: fs, macros, rt-multi-thread, io-util) |
| `reqwest` | HTTP client (default-features: false, with rustls) |
| `anyhow` | Error handling |
| `tracing` / `tracing-subscriber` | Structured logging |
| `chrono` | Date/time with serde support |
| `indicatif` | Progress bars |
| `clap` | CLI argument parsing (features: derive) |
| `serde` / `serde_json` | Serialization |

## CLI Usage Pattern

```sh
siwi-download <URL> [OPTIONS]
```

| Option | Short | Description |
|--------|-------|-------------|
| `--url` | `-u` | URL to download |
| `--output` | `-o` | Output directory |
| `--filename` | `-f` | Custom filename |
| `--progress` | `-P` | Show progress bar |
| `--proxy` | `-p` | HTTP proxy URL |
| `--verbose` | `-v` | Verbose logging |
| `--json` | `-j` | JSON output format |

## Library API Usage

```rust
use siwi_download::{
  download::{Download, DownloadOptions},
  error::AnyResult,
};

let download = Download::new(storage_path);
download.auto_create_storage_path().await?;
let report = download.download(url, options).await?;
```

## Testing

- Place tests in `#[cfg(test)]` modules within the same file
- Use `AnyResult<()>` return type for tests
- Use descriptive test names: `do_get_file_name_from_url` not `test1`

## Common Patterns

### Breakpoint Continuation (Resume)
- Uses HTTP Range header to request partial content
- Checks file size against Content-Length
- Appends to existing file if partial download exists

### Progress Tracking
- Uses `indicatif` ProgressBar and ProgressStyle
- Reports: bytes downloaded, speed, ETA

### HTTP Redirect Handling
- Custom redirect logic via `reqwest::redirect::Policy`
- Maximum redirect limit enforced

## Version and Edition

- Rust Edition: 2024
- Minimum Rust Version: 1.85
