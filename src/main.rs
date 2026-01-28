//! siwi-download CLI
//!
//! A command-line interface for the siwi-download library.
//!
//! ## Usage
//!
//! ```sh
//! # Download a file (simplified)
//! siwi-download https://example.com/file.zip
//!
//! # Download with options
//! siwi-download -u https://example.com/file.zip -o ./downloads -P
//!
//! # Download with custom filename and proxy
//! siwi-download https://example.com/file.zip -f myfile.zip -p http://proxy:8080
//!
//! # Verbose mode for debugging
//! siwi-download https://example.com/file.zip -v
//! ```
//!
//! For more information, run: `siwi-download --help`

use clap::{Arg, ArgAction, Command};
use siwi_download::download::Download;
use siwi_download::download::DownloadOptions;
use siwi_download::error::AnyResult;
use tracing::Level;
use tracing_subscriber::FmtSubscriber;

#[tokio::main]
async fn main() -> AnyResult<()> {
  let matches = Command::new("siwi-download")
    .author("Mankong, siwilizhao")
    .version("1.0.0")
    .about("Downloader with breakpoint continuation support")
    .arg(
      Arg::new("url")
        .help("URL to download (positional or via -u/--url)")
        .short('u')
        .long("url")
        .required(false)
        .default_value(""),
    )
    .arg(
      Arg::new("output")
        .help("Output directory for downloaded file")
        .short('o')
        .long("output")
        .default_value("."),
    )
    .arg(
      Arg::new("filename")
        .help("Custom filename for the downloaded file")
        .short('f')
        .long("filename")
        .required(false),
    )
    .arg(
      Arg::new("progress")
        .help("Show download progress bar")
        .short('P')
        .long("progress")
        .default_value("true")
        .action(ArgAction::SetTrue),
    )
    .arg(
      Arg::new("proxy")
        .help("HTTP proxy")
        .short('p')
        .long("proxy")
        .required(false),
    )
    .arg(
      Arg::new("verbose")
        .help("Verbose output")
        .short('v')
        .long("verbose")
        .default_value("false")
        .action(ArgAction::SetTrue),
    )
    .get_matches();

  let url = matches.get_one::<String>("url").unwrap().clone();
  if url.is_empty() {
    eprintln!("Error: URL is required. Use `-u <url>` or provide URL as first argument.");
    std::process::exit(1);
  }

  let output = matches.get_one::<String>("output").unwrap().clone();
  let filename = matches
    .get_one::<String>("filename")
    .cloned()
    .filter(|s| !s.is_empty());
  let progress = matches.get_flag("progress");
  let proxy = matches.get_one::<String>("proxy").cloned();
  let verbose = matches.get_flag("verbose");

  let log_level = if verbose { Level::DEBUG } else { Level::INFO };

  let subscriber = FmtSubscriber::builder().with_max_level(log_level).finish();
  tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

  let mut options = DownloadOptions::default();
  options.set_show_progress(progress);

  if let Some(filename) = filename {
    options.set_file_name(filename);
  }

  if let Some(proxy) = proxy {
    options.set_proxy(proxy);
  }

  let download = Download::new(&output);
  download.auto_create_storage_path().await?;
  let report = download.download(&url, options).await?;
  println!("{:#?}", report);

  Ok(())
}
