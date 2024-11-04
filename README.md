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
  <!-- Ci -->
  <a href="https://github.com/rs-videos/siwi-download/actions">
    <img src="https://github.com/rs-videos/siwi-download/workflows/Rust/badge.svg"
      alt="github actions" />
  </a>
</div>

<div align="center">
  <h3>
    <a href="https://docs.rs/siwi-download">
      API Docs
    </a>
  </h3>
</div>

Siwi Download is a downloader build on tokio and reqwest.

## Example

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

- Write a CLI tool

```rust
use siwi_download::download::Download;
use siwi_download::download::DownloadOptions;
use siwi_download::error::AnyResult;
#[tokio::main]
async fn main() -> AnyResult<()> {
  let args: Vec<String> = std::env::args().collect();
  let storage_path = std::env::current_dir()?;
  let storage_path = storage_path.to_str().unwrap_or("");

  if let Some(url) = args.get(1) {
    let mut options = DownloadOptions::default();
    options.set_show_progress(true);
    let download = Download::new(storage_path);
    let report = download.download(url, options).await?;
    println!("{:?}", report);
  }
  Ok(())
}

```
