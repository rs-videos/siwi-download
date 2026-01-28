//! # siwi-download
//!
//! A high-performance file downloader library built on Tokio and Reqwest,
//! featuring breakpoint continuation, progress tracking, and async download capabilities.
//!
//! ## Features
//!
//! - 🚀 **Async Download**: Built on Tokio for high-performance concurrent downloads
//! - 📊 **Progress Tracking**: Visual progress bar support with `indicatif`
//! - 🔄 **Resume Support**: Breakpoint continuation for interrupted downloads
//! - 🌐 **Proxy Support**: HTTP/HTTPS proxy support
//! - 📁 **Flexible Storage**: Custom output directory and filename support
//! - 🔧 **Builder Pattern**: Fluent API for configuration
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use siwi_download::download::{Download, DownloadOptions};
//! use tracing::Level;
//! use tracing_subscriber::FmtSubscriber;
//!
//! #[tokio::main]
//! async fn main() -> siwi_download::error::AnyResult<()> {
//!     let subscriber = FmtSubscriber::builder()
//!         .with_max_level(Level::INFO)
//!         .finish();
//!     tracing::subscriber::set_global_default(subscriber)
//!         .expect("setting default subscriber failed");
//!
//!     let download = Download::new("./downloads");
//!     download.auto_create_storage_path().await?;
//!
//!     let report = download
//!         .download("https://example.com/file.zip", DownloadOptions::default())
//!         .await?;
//!
//!     println!("Download completed: {:?}", report);
//!     Ok(())
//! }
//! ```
//!
//! ## CLI Usage
//!
//! Install the CLI tool:
//!
//! ```sh
//! cargo install siwi-download
//! ```
//!
//! Download a file:
//!
//! ```sh
//! siwi-download -u https://example.com/file.zip -o ./downloads -P
//! ```

pub mod download;
pub mod error;
pub mod utils;
