<h1 align="center">Siwi Download</h1>
<div align="center">
  <strong>
    Download file
  </strong>
</div>

<br />


<div align="center">
  <!-- Crates version -->
  <a href="https://crates.io/crates/siwi-download">
    <img src="https://img.shields.io/crates/v/siwi-download.svg?style=flat-square"
    alt="Crates.io version" />
  </a>
  <!-- License -->
  <a href="https://crates.io/crates/siwi-download">
    <img src="https://img.shields.io/crates/l/siwi-download"
      alt="License" />
  </a>
  <!-- Downloads -->
  <a href="https://crates.io/crates/siwi-download">
    <img src="https://img.shields.io/crates/d/siwi-download.svg?style=flat-square"
      alt="Download" />
  </a>
  <!-- docs.rs docs -->
  <a href="https://docs.rs/siwi-download">
    <img src="https://img.shields.io/badge/docs-latest-blue.svg?style=flat-square"
      alt="docs.rs docs" />
  </a>
  <!-- CI -->
  <a href="https://github.com/rs-videos/siwi-download/actions">
    <img src="https://github.com/rs-videos/siwi-download/workflows/CI/badge.svg"
      alt="CI status" />
  </a>
</div>

<div align="center">
  <h3>
    <a href="https://docs.rs/siwi-download">API Docs</a>
    <span>&nbsp;&nbsp;•&nbsp;&nbsp;</span>
    <a href="CHANGELOG.md">Changelog</a>
    <span>&nbsp;&nbsp;•&nbsp;&nbsp;</span>
    <a href="AGENTS.md">Contributing</a>
    <span>&nbsp;&nbsp;•&nbsp;&nbsp;</span>
    <a href="docs/README.md">Articles</a>
    <span>&nbsp;&nbsp;•&nbsp;&nbsp;</span>
    <a href="docs/ROADMAP.md">Roadmap</a>
  </h3>
</div>

Siwi Download is a downloader built on tokio and reqwest with breakpoint
continuation support.

## Features

- 🚀 **Async download** - Built on tokio for high performance
- 📊 **Progress bar** - Visual download progress
- 🔄 **Resume support** - Breakpoint continuation for interrupted downloads
- 🌐 **Proxy support** - HTTP/HTTPS proxy support
- ⏱️ **Timeouts** - Request (60s) and connect (10s) timeouts built in
- 📁 **Custom paths** - Specify output directory and filename
- 📄 **JSON output** - Machine-readable report output
- 🔧 **Library API** - Use as a Rust library in your project

## Requirements

- Rust **1.85+** (edition 2024). The toolchain is pinned via
  `rust-toolchain.toml`; CI verifies both `stable` and the MSRV.

## Install

```sh
cargo install siwi-download
```

## CLI Usage

```sh
siwi-download <URL> [OPTIONS]
```

### Options

| Option | Short | Description | Default |
|--------|-------|-------------|---------|
| `<url>` | positional | URL to download (first argument) | Required |
| `--url` | `-u` | URL to download (alternative) | - |
| `--output` | `-o` | Output directory | Current directory |
| `--filename` | `-f` | Custom filename | Auto-extracted from URL |
| `--progress` | `-P` | Show progress bar | `false` |
| `--proxy` | `-p` | HTTP proxy URL | None |
| `--verbose` | `-v` | Verbose logging | `false` |
| `--json` | `-j` | Output report in JSON format | `false` |
| `--help` | `-h` | Show help | - |
| `--version` | `-V` | Show version | - |

### Examples

**Basic download (simplified):**
```sh
siwi-download https://nodejs.org/dist/v22.11.0/node-v22.11.0.pkg
```

**Basic download (with flag):**
```sh
siwi-download -u https://nodejs.org/dist/v22.11.0/node-v22.11.0.pkg
```

**Download to specific directory with progress:**
```sh
siwi-download https://example.com/file.zip -o /tmp/downloads -P
```

**Download with custom filename:**
```sh
siwi-download https://example.com/download -f my-custom-name.zip
```

**Download through proxy:**
```sh
siwi-download https://large-file.iso -p http://127.0.0.1:7890 -P
```

**Verbose mode for debugging:**
```sh
siwi-download https://example.com/file.zip -v
```

**JSON output for scripting:**
```sh
siwi-download https://example.com/file.zip -j
```

Logs go to **stderr**, so `--json` stdout is safe to pipe into `jq`.

**Verify the download against a published checksum:**
```sh
siwi-download https://example.com/file.iso --checksum sha256:abc123...
```
A mismatch fails the download (`download_status: "Error"`, exit code 10).

**Cap the download speed:**
```sh
siwi-download https://example.com/file.iso --max-speed 10M
```
1024-based `K`/`M`/`G` suffixes (optional trailing `B`).

**Skip unchanged files (conditional request):**
```sh
siwi-download https://example.com/dataset.tar.gz --if-modified
```
Sends `If-Modified-Since` derived from the local file's mtime; a `304`
answer skips the body download entirely.

**Output Example (JSON format):**
```json
{
  "url": "https://example.com/file.zip",
  "file_name": "file.zip",
  "origin_file_name": "file.zip",
  "storage_path": "/downloads",
  "file_path": "/downloads/file.zip",
  "file_size": 1048576,
  "range_from": 0,
  "download_start_at": "2026-07-21T10:30:00Z",
  "download_end_at": "2026-07-21T10:30:05Z",
  "download_status": "Complete",
  "head_status": 200,
  "resp_status": 206,
  "time_used": 5,
  "msg": null
}
```

> The `headers` field is intentionally skipped during serialization.

## Configuration

Defaults can be persisted in a TOML config file. The location is
platform-dependent unless `--config <path>` names one explicitly:

| Platform | Path |
|---|---|
| Linux | `$XDG_CONFIG_HOME/siwi-download/config.toml` or `~/.config/siwi-download/config.toml` |
| macOS | `~/Library/Application Support/siwi-download/config.toml` |
| Windows | `%APPDATA%\siwi-download\config.toml` |

```toml
[default]
output = "./downloads"
progress = true
max_speed = "20M"     # 1024-based K/M/G suffixes

[proxy]
url = "http://127.0.0.1:7890"
```

Unknown keys are rejected so typos fail loudly. Precedence, highest first:

1. CLI flags (`-o`, `-p`, `--max-speed`, `-P`)
2. Environment variables (`SIWI_DOWNLOAD_OUTPUT`, `SIWI_DOWNLOAD_PROXY`,
   `SIWI_DOWNLOAD_MAX_SPEED`, `SIWI_DOWNLOAD_PROGRESS`)
3. The config file
4. Built-in defaults

## Batch Downloads

Run a whole set of downloads from a TOML manifest with a concurrency cap and
`depends_on` ordering:

```toml
# batch.toml
concurrent = 2
state_file = "./state.json"   # optional: persist definitions for resuming

[[tasks]]
id = "model"
url = "https://example.com/model.bin"
output = "./models"

[[tasks]]
id = "dataset"
url = "https://example.com/dataset.tar.gz"
output = "./data"
depends_on = ["model"]        # waits until `model` completes
```

```sh
siwi-download --batch batch.toml
```

Failed dependencies cascade as *skipped* (never silently ignored); the
command exits `10` when anything failed. Re-running an interrupted batch
resumes unfinished files via breakpoint continuation.

## Library Usage

> cargo run --example download

```rust
use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
use siwi_download::{
  download::{Download, DownloadOptions},
  error::AnyResult,
};
use tracing::Level;
use tracing_subscriber::FmtSubscriber;

#[tokio::main]
async fn main() -> AnyResult<()> {
  let subscriber = FmtSubscriber::builder()
    .with_max_level(Level::INFO)
    .finish();
  tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");
  let url = "https://nodejs.org/dist/v22.11.0/node-v22.11.0.pkg";
  let mut storage_path = std::env::current_dir()?;
  storage_path.push("storage");
  let storage_path = storage_path.to_str().unwrap();
  let mut options = DownloadOptions::default();
  let mut headers = HeaderMap::new();
  headers.insert(USER_AGENT, HeaderValue::from_str("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36")?);
  options
    .set_headers(headers)
    .set_file_name("node-v22.11.0.pkg")
    .set_show_progress(true);

  let download = Download::new(storage_path);
  download.auto_create_storage_path().await?;

  let report = download.download(url, options).await?;
  println!("report {:#?}", report);
  Ok(())
}
```

See [examples/download.rs](examples/download.rs) for complete library examples.
