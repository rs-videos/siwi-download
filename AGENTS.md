# AGENTS.md - siwi-download

This document provides guidelines for AI agents working on this codebase.

## Build / Lint / Test Commands

```bash
# Build the project
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
- **Snake case** for functions and variables: `get_file_name_from_url()`
- **PascalCase** for types, traits, and structs: `DownloadOptions`, `DownloadReport`
- **CamelCase** for enum variants: `DownloadStatus::Create`, `DownloadStatus::Append`
- **SCREAMING_SNAKE_CASE** for constants: Not used in this codebase
- Prefix private helper functions with underscore: `fn _internal_helper()`

### Imports and Module Organization
```rust
// Group imports by crate type
use crate::{
  error::AnyResult,
  utils::{get_file_name_from_url, get_file_size},
};
use chrono::{DateTime, Utc};
use reqwest::header::CONTENT_LENGTH;
use tokio::{fs, io::AsyncWriteExt};

// External crates first, then std, then crate
```

### Error Handling
- Use `anyhow` for application errors
- Define type aliases at top of error module:
```rust
pub type AnyError = anyhow::Error;
pub type AnyResult<T> = anyhow::Result<T, AnyError>;
```
- Propagate errors with `?` operator
- Use `tracing` for error logging: `error!("message {:?}", err)`

### Async Patterns
- Use `tokio` runtime with `#[tokio::main]` for main binaries
- Prefer `async/.await` over blocking I/O
- Use `tokio::fs` for file operations in async context
- Include retry logic with exponential backoff for network operations

### Struct and Builder Patterns
- Use builder pattern for options with setter methods:
```rust
impl<'a> DownloadOptions<'a> {
  pub fn set_file_name<S: Into<Cow<'a, str>>>(&mut self, file_name: S) -> &mut Self {
    self.maybe_file_name = Some(file_name.into());
    self
  }
}
```
- Use `Default` trait for default struct initialization
- Use `Cow<'a, str>` for lifetime-efficient string handling

### Documentation
- Document public APIs with `///` doc comments
- Include examples in doc comments where helpful
- Mark docs-specific code with `#[cfg(feature = "docs")]`

### Testing
- Place tests in `#[cfg(test)]` modules within the same file
- Use `AnyResult<()>` return type for tests that may fail
- Use descriptive test names: `do_get_file_name_from_url` not `test1`

### Logging
- Use `tracing` crate for structured logging
- Use appropriate log levels: `info!`, `error!`, `warn!`
- Include relevant context in log messages

### Key Dependencies
- `tokio` - async runtime (features: fs, macros, rt-multi-thread, io-util)
- `reqwest` - HTTP client (default-features: false, with rustls)
- `anyhow` - error handling
- `tracing` / `tracing-subscriber` - logging
- `chrono` - date/time with serde support
- `indicatif` - progress bars
- `clap` - CLI argument parsing (features: derive)
- `serde` / `serde_json` - serialization

### File Structure
```
src/
  lib.rs      - Main library entry, exports modules
  main.rs     - Binary CLI entry point
  error.rs    - Error type definitions
  download.rs - Core download logic and types
  utils.rs    - Utility functions
examples/
  cli.rs      - CLI example
  download.rs - Download API example
```
