//! CLI example for siwi-download
//!
//! This example demonstrates how to use the siwi-download CLI tool.
//! Run with: cargo run --example cli
//!
//! ```bash
//! # Basic download
//! cargo run --example cli -u https://example.com/file.zip
//!
//! # Download with progress bar
//! cargo run --example cli -u https://example.com/file.zip -P
//!
//! # Download to specific directory
//! cargo run --example cli -u https://example.com/file.zip -o /tmp/downloads
//!
//! # Download with custom filename
//! cargo run --example cli -u https://example.com/file.zip -f myfile.zip
//!
//! # Download through proxy
//! cargo run --example cli -u https://example.com/large-file.iso -p http://127.0.0.1:7890 -P
//! ```

use siwi_download::download::Download;
use siwi_download::download::DownloadOptions;
use siwi_download::error::AnyResult;
use tracing::{Level, info};
use tracing_subscriber::FmtSubscriber;

/// Simple CLI downloader using siwi-download library
///
/// This example shows how to use the Download API directly in a CLI application.
/// For a full-featured CLI, use the binary built from main.rs with clap.
#[tokio::main]
async fn main() -> AnyResult<()> {
  // Configure logging
  let subscriber = FmtSubscriber::builder()
    .with_max_level(Level::INFO)
    .finish();
  tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

  // Setup download
  let binding = std::env::current_dir()?;
  let storage_path = binding.to_str().unwrap_or(".");

  let url = "https://nodejs.org/dist/v22.11.0/node-v22.11.0.pkg";

  // Configure download options
  let mut options = DownloadOptions::default();
  options
    .set_file_name("node-v22.11.0.pkg")
    .set_show_progress(true);

  // Create downloader and execute
  let download = Download::new(storage_path);
  download.auto_create_storage_path().await?;

  let report = download.download(url, options).await?;

  // Print download report
  info!("Download completed: {:#?}", report);

  Ok(())
}
