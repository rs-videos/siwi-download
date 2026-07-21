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
//! siwi-download https://example.com/file.zip -o ./downloads -P
//!
//! # Download with custom filename and proxy
//! siwi-download https://example.com/file.zip -f myfile.zip -p http://proxy:8080
//!
//! # Verbose mode for debugging
//! siwi-download https://example.com/file.zip -v
//!
//! # JSON output for scripting
//! siwi-download https://example.com/file.zip -j
//! ```
//!
//! For more information, run: `siwi-download --help`

use clap::{Arg, ArgAction, Command};
use serde_json::to_string_pretty;
use siwi_download::download::{Download, DownloadOptions};
use siwi_download::error::AnyResult;
use tracing::Level;
use tracing_subscriber::FmtSubscriber;

#[tokio::main]
async fn main() -> AnyResult<()> {
  let matches = Command::new("siwi-download")
    .author("Mankong, siwilizhao")
    .version(env!("CARGO_PKG_VERSION"))
    .about("Downloader with breakpoint continuation support")
    .arg(
      Arg::new("url")
        .help("URL to download (positional, or use --url)")
        .index(1)
        .required(false),
    )
    .arg(
      Arg::new("url_flag")
        .help("URL to download (alternative to the positional argument)")
        .long("url")
        .short('u')
        .required(false),
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
        .action(ArgAction::SetTrue),
    )
    .arg(
      Arg::new("json")
        .help("Output report in JSON format")
        .short('j')
        .long("json")
        .action(ArgAction::SetTrue),
    )
    .get_matches();

  // URL can be provided either positionally or via -u/--url.
  let url = matches
    .get_one::<String>("url")
    .or_else(|| matches.get_one::<String>("url_flag"))
    .cloned()
    .filter(|s| !s.is_empty())
    .unwrap_or_else(|| {
      eprintln!("Error: URL is required. Pass it as the first argument or via --url/-u.");
      std::process::exit(1);
    });

  let output = matches.get_one::<String>("output").cloned().unwrap();
  let filename = matches
    .get_one::<String>("filename")
    .cloned()
    .filter(|s| !s.is_empty());
  let progress = matches.get_flag("progress");
  let proxy = matches.get_one::<String>("proxy").cloned();
  let verbose = matches.get_flag("verbose");
  let json_output = matches.get_flag("json");

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

  if json_output {
    println!("{}", to_string_pretty(&report)?);
  } else {
    println!("{:#?}", report);
  }

  Ok(())
}
