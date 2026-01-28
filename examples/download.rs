//! Library API example for siwi-download
//!
//! This example demonstrates how to use siwi-download as a library
//! in your own Rust application.
//!
//! Run with: cargo run --example download

use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
use siwi_download::download::{Download, DownloadOptions};
use siwi_download::error::AnyResult;
use tracing::{Level, info};
use tracing_subscriber::FmtSubscriber;

/// Example: Download a file using the siwi-download library API
///
/// This example shows:
/// - Setting up a custom User-Agent header
/// - Configuring download options with the builder pattern
/// - Using storage path management
/// - Handling the download report
#[tokio::main]
async fn main() -> AnyResult<()> {
  // Initialize tracing for logging
  let subscriber = FmtSubscriber::builder()
    .with_max_level(Level::INFO)
    .finish();
  tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

  // Configuration
  let url = "https://nodejs.org/dist/v22.11.0/node-v22.11.0.pkg";

  // Create storage directory path
  let mut storage_path = std::env::current_dir()?;
  storage_path.push("storage");
  let storage_path = storage_path.to_str().unwrap();

  // Configure download options using builder pattern
  let mut options = DownloadOptions::default();

  // Set custom headers (e.g., User-Agent)
  let mut headers = HeaderMap::new();
  headers.insert(
    USER_AGENT,
    HeaderValue::from_str("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36")?,
  );
  options
    .set_headers(headers)
    .set_file_name("node-v22.11.0.pkg")
    .set_show_progress(true);

  // Create downloader instance
  let download = Download::new(storage_path);

  // Ensure storage directory exists
  download.auto_create_storage_path().await?;

  // Execute download
  let report = download.download(url, options).await?;

  // Log the download report
  info!("Download report: {:#?}", report);

  // Print as JSON for programmatic access
  let json = serde_json::to_string_pretty(&report)?;
  info!("Report JSON:\n{}", json);

  Ok(())
}
