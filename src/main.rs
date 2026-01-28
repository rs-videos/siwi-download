//! siwi-download CLI
//!
//! A command-line interface for the siwi-download library.
//!
//! ## Usage
//!
//! ```sh
//! # Download a file
//! siwi-download -u https://example.com/file.zip
//!
//! # Download with progress bar to specific directory
//! siwi-download -u https://example.com/file.zip -o ./downloads -P
//!
//! # Download with custom filename and proxy
//! siwi-download -u https://example.com/file.zip -f myfile.zip -p http://proxy:8080
//!
//! # Verbose mode for debugging
//! siwi-download -u https://example.com/file.zip -v
//! ```
//!
//! For more information, run: `siwi-download --help`

use clap::Parser;
use siwi_download::download::Download;
use siwi_download::download::DownloadOptions;
use siwi_download::error::AnyResult;
use tracing::Level;
use tracing_subscriber::FmtSubscriber;

/// A simple file downloader with breakpoint continuation support
#[derive(Parser, Debug)]
#[command(name = "siwi-download")]
#[command(author, version, about, long_about = None)]
struct Args {
  /// URL to download
  #[arg(short, long)]
  url: String,

  /// Output directory for downloaded file (default: current directory)
  #[arg(short, long, default_value = ".")]
  output: String,

  /// Custom filename for the downloaded file
  #[arg(short, long)]
  filename: Option<String>,

  /// Show download progress bar
  #[arg(short = 'P', long, default_value = "true")]
  progress: bool,

  /// HTTP proxy (e.g., http://127.0.0.1:7890)
  #[arg(short, long)]
  proxy: Option<String>,

  /// Verbose output
  #[arg(short, long, default_value = "false")]
  verbose: bool,
}

#[tokio::main]
async fn main() -> AnyResult<()> {
  let args = Args::parse();

  let log_level = if args.verbose {
    Level::DEBUG
  } else {
    Level::INFO
  };

  let subscriber = FmtSubscriber::builder().with_max_level(log_level).finish();
  tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

  let mut options = DownloadOptions::default();
  options
    .set_show_progress(args.progress)
    .set_file_name(args.filename.unwrap_or_default());

  if let Some(proxy) = args.proxy {
    options.set_proxy(proxy);
  }

  let download = Download::new(&args.output);
  download.auto_create_storage_path().await?;

  let report = download.download(&args.url, options).await?;
  println!("{:#?}", report);

  Ok(())
}
