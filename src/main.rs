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
//! # Verify the download against a published checksum
//! siwi-download https://example.com/file.iso --checksum sha256:abc123...
//!
//! # Cap the download at 10 MB/s (1024-based suffixes: K, M, G)
//! siwi-download https://example.com/file.iso --max-speed 10M
//!
//! # Skip the download when the server has nothing newer than the local copy
//! siwi-download https://example.com/file.zip --if-modified
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
use siwi_download::download::checksum;
use siwi_download::download::{Download, DownloadOptions};
use siwi_download::error::AnyResult;
use siwi_download::utils::get_file_name_from_url;
use tracing::Level;
use tracing_subscriber::FmtSubscriber;

mod config;

#[tokio::main]
async fn main() -> AnyResult<()> {
  let matches = Command::new("siwi-download")
    .author("Mankong, siwilizhao")
    .version(env!("CARGO_PKG_VERSION"))
    .about("Downloader with breakpoint continuation support")
    .arg_required_else_help(true)
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
        .help("Output directory for downloaded file [default: .]")
        .short('o')
        .long("output"),
    )
    .arg(
      Arg::new("config")
        .help("Path to config file [default: <platform config dir>/siwi-download/config.toml]")
        .long("config"),
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
      Arg::new("checksum")
        .help("Verify the download against this checksum (e.g. sha256:<hex>, sha1:<hex>, md5:<hex>)")
        .long("checksum")
        .required(false),
    )
    .arg(
      Arg::new("max_speed")
        .help("Cap the average download speed, e.g. 500K, 10M, 1G (bytes/second, 1024-based)")
        .long("max-speed")
        .required(false),
    )
    .arg(
      Arg::new("if_modified")
        .help("Skip the download when the server has nothing newer than the local copy (sends If-Modified-Since using the local file's mtime)")
        .long("if-modified")
        .action(ArgAction::SetTrue),
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

  // URL can be provided either positionally or via -u/--url. Handled here
  // (rather than via clap's `required`) so users can combine `-u` with other
  // flags in any order. `arg_required_else_help` above already covers the
  // "no args at all" case.
  let url = matches
    .get_one::<String>("url")
    .or_else(|| matches.get_one::<String>("url_flag"))
    .cloned()
    .filter(|s| !s.is_empty())
    .unwrap_or_else(|| {
      eprintln!(
        "Error: URL is required. Pass it as the first argument or via --url/-u.\nUse --help for usage."
      );
      std::process::exit(2);
    });

  let output_flag = matches
    .get_one::<String>("output")
    .cloned()
    .filter(|s| !s.is_empty());
  let filename = matches
    .get_one::<String>("filename")
    .cloned()
    .filter(|s| !s.is_empty());
  let progress_flag = matches.get_flag("progress");
  let proxy_flag = matches
    .get_one::<String>("proxy")
    .cloned()
    .filter(|s| !s.is_empty());
  let verbose = matches.get_flag("verbose");
  let json_output = matches.get_flag("json");
  let checksum_spec = matches
    .get_one::<String>("checksum")
    .cloned()
    .filter(|s| !s.is_empty());
  let max_speed_flag = matches
    .get_one::<String>("max_speed")
    .cloned()
    .filter(|s| !s.is_empty());
  let if_modified = matches.get_flag("if_modified");
  let config_path = matches
    .get_one::<String>("config")
    .cloned()
    .filter(|s| !s.is_empty());

  // Layer the configuration: CLI flag > env var > config file > default.
  // The config file must parse cleanly even when only defaults are used, so
  // typos never silently change behavior.
  let cfg = config::load(config_path.as_deref())?;
  let resolved = config::resolve(cfg.as_ref(), &|k| std::env::var(k).ok())?;

  let output = output_flag
    .or(resolved.output)
    .unwrap_or_else(|| ".".to_owned());
  let progress = progress_flag || resolved.progress.unwrap_or(false);
  let proxy = proxy_flag.or(resolved.proxy);

  // Parse --checksum up front so a malformed spec fails before any I/O.
  let parsed_checksum = match checksum_spec.as_deref() {
    Some(spec) => {
      let (algo, expected) = checksum::parse_spec(spec)?;
      Some((algo, expected))
    }
    None => None,
  };

  let max_speed = match max_speed_flag.as_deref() {
    Some(spec) => Some(config::parse_speed_spec(spec)?),
    None => resolved.max_speed_bytes,
  };
  if max_speed == Some(0) {
    return Err(anyhow::anyhow!("speed must be greater than zero"));
  }

  let log_level = if verbose { Level::DEBUG } else { Level::INFO };

  // Logs go to stderr so `--json` stdout stays clean for piping into jq.
  let subscriber = FmtSubscriber::builder()
    .with_max_level(log_level)
    .with_writer(std::io::stderr)
    .finish();
  tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

  let mut options = DownloadOptions::default();
  options.set_show_progress(progress);

  if let Some(filename) = filename {
    options.set_file_name(filename);
  }

  if let Some(proxy) = proxy {
    options.set_proxy(proxy);
  }

  if let Some((algo, expected)) = parsed_checksum {
    options.set_checksum(algo, expected);
  }

  if let Some(bytes) = max_speed {
    options.set_max_speed(bytes);
  }

  let download = Download::new(&output);
  download.auto_create_storage_path().await?;

  // `--if-modified` derives If-Modified-Since from the local file's mtime:
  // a completed download sets mtime to "now", so re-running with the flag
  // skips the body unless the remote copy is newer.
  if if_modified {
    let file_name = options
      .maybe_file_name
      .clone()
      .or_else(|| get_file_name_from_url(&url).ok().filter(|s| !s.is_empty()))
      .unwrap_or_default();
    let path = std::path::Path::new(&output).join(&file_name);
    if let Ok(meta) = tokio::fs::metadata(&path).await
      && let Ok(modified) = meta.modified()
      && let Ok(modified) = modified.duration_since(std::time::UNIX_EPOCH)
    {
      use chrono::TimeZone;
      let since = chrono::Utc
        .timestamp_opt(modified.as_secs() as i64, 0)
        .single()
        .unwrap_or_else(chrono::Utc::now);
      options.set_if_modified_since(since);
    }
  }

  let report = download.download(&url, options).await?;

  if json_output {
    println!("{}", to_string_pretty(&report)?);
  } else {
    println!("{:#?}", report);
  }

  // Non-zero exit when the download is not fully healthy, so scripts can
  // branch on the result.
  if report.download_status == Some(siwi_download::download::DownloadStatus::Error) {
    std::process::exit(10);
  }

  Ok(())
}
