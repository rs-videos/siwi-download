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
use serde::Deserialize;
use serde_json::to_string_pretty;
use siwi_download::download::checksum;
use siwi_download::download::observability::{AccessLogHook, Metrics};
use siwi_download::download::{CommandHook, Download, DownloadHook, DownloadOptions};
use siwi_download::error::AnyResult;
use siwi_download::utils::get_file_name_from_url;
use std::sync::Arc;
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
      Arg::new("batch")
        .help("TOML batch manifest: [default] output/progress/max_speed, concurrent = N, [[tasks]] id/url/output/file_name/depends_on. Incompatible with the positional url, -u, --stdout, and -f")
        .long("batch")
        .conflicts_with_all(["url", "url_flag", "stdout", "filename"]),
    )
    .arg(
      Arg::new("dry_run")
        .help("Probe the URL (HEAD) and print the report without downloading")
        .long("dry-run")
        .action(ArgAction::SetTrue)
        .conflicts_with_all(["stdout", "on_complete", "batch", "checksum"]),
    )
    .arg(
      Arg::new("metrics_file")
        .help("Write Prometheus text metrics for this run to this file")
        .long("metrics-file"),
    )
    .arg(
      Arg::new("access_log")
        .help("Append a JSON access log line per download to this file")
        .long("access-log"),
    )
    .arg(
      Arg::new("on_complete")
        .help("Run this shell command when the download finishes. Context via env vars: SIWI_FILE_PATH, SIWI_FILE_SIZE, SIWI_URL, SIWI_STATUS, SIWI_DOWNLOAD_STATUS")
        .long("on-complete"),
    )
    .arg(
      Arg::new("stdout")
        .help("Stream the body to stdout instead of a file (binary-safe; the report goes to stderr; incompatible with -o, -f, and -j)")
        .long("stdout")
        .action(ArgAction::SetTrue)
        .conflicts_with_all(["output", "filename", "json"]),
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

  let verbose = matches.get_flag("verbose");
  let json_output = matches.get_flag("json");
  let batch_path = matches
    .get_one::<String>("batch")
    .cloned()
    .filter(|s| !s.is_empty());

  let log_level = if verbose { Level::DEBUG } else { Level::INFO };

  // Logs go to stderr so `--json` stdout stays clean for piping into jq.
  let subscriber = FmtSubscriber::builder()
    .with_max_level(log_level)
    .with_writer(std::io::stderr)
    .finish();
  tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

  // Batch mode runs a whole manifest and exits; the single-URL path below
  // never runs.
  if let Some(manifest_path) = batch_path {
    let exit_code = run_batch(&manifest_path, verbose).await?;
    if exit_code != 0 {
      std::process::exit(exit_code.into());
    }
    return Ok(());
  }

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
  let checksum_spec = matches
    .get_one::<String>("checksum")
    .cloned()
    .filter(|s| !s.is_empty());
  let max_speed_flag = matches
    .get_one::<String>("max_speed")
    .cloned()
    .filter(|s| !s.is_empty());
  let if_modified = matches.get_flag("if_modified");
  let stdout_mode = matches.get_flag("stdout");
  let dry_run = matches.get_flag("dry_run");
  let metrics_file = matches
    .get_one::<String>("metrics_file")
    .cloned()
    .filter(|s| !s.is_empty());
  let access_log = matches
    .get_one::<String>("access_log")
    .cloned()
    .filter(|s| !s.is_empty());
  let on_complete = matches
    .get_one::<String>("on_complete")
    .cloned()
    .filter(|s| !s.is_empty());
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

  if let Some(cmd) = on_complete {
    options.add_hook(Arc::new(CommandHook::new(cmd)));
  }

  // Observability hooks compose like any other hook.
  let metrics = metrics_file.clone().map(|_| Arc::new(Metrics::new()));
  if let Some(m) = metrics.as_ref() {
    options.add_hook(m.clone() as Arc<dyn DownloadHook>);
  }
  if let Some(path) = access_log.as_ref() {
    match AccessLogHook::to_file(path) {
      Ok(hook) => options.add_hook(Arc::new(hook)),
      Err(e) => return Err(anyhow::anyhow!("{e}")),
    };
  }

  // Ctrl+C sets the cooperative cancel flag: the current chunk finishes,
  // the file is flushed, and the partial download remains resumable.
  let cancel_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
  options.set_cancel(cancel_flag.clone());
  {
    let cancel_flag = cancel_flag.clone();
    tokio::spawn(async move {
      if tokio::signal::ctrl_c().await.is_ok() {
        eprintln!("\nreceived Ctrl+C: finishing current chunk, progress is resumable");
        cancel_flag.store(true, std::sync::atomic::Ordering::Relaxed);
      }
    });
  }

  let download = Download::new(&output);
  download.auto_create_storage_path().await?;

  if dry_run {
    let report = download.probe(&url, &options).await?;
    if json_output {
      println!("{}", to_string_pretty(&report)?);
    } else {
      println!("{report:#?}");
    }
    return Ok(());
  }

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

  let report = if stdout_mode {
    // Binary-safe pipe: body to stdout, report to stderr. `--if-modified`
    // does not apply (there is no local file to compare against).
    use siwi_download::download::sink::StdoutSink;
    let mut sink = StdoutSink;
    let report = download.stream(&url, options, &mut sink).await?;
    eprintln!("{report:#?}");
    report
  } else {
    download.download(&url, options).await?
  };

  if let Some(m) = metrics.as_ref() {
    if let Err(e) = std::fs::write(
      metrics_file.as_deref().unwrap_or("./siwi-metrics.prom"),
      m.render_prometheus(),
    ) {
      eprintln!("warning: could not write metrics file: {e}");
    }
  }

  if json_output {
    println!("{}", to_string_pretty(&report)?);
  } else if !stdout_mode {
    println!("{:#?}", report);
  }

  // Non-zero exit when the download is not fully healthy, so scripts can
  // branch on the result.
  if report.download_status == Some(siwi_download::download::DownloadStatus::Error) {
    std::process::exit(10);
  }

  Ok(())
}

/// Runs a batch manifest: parse, validate, execute, summarize.
///
/// Returns the process exit code (0 on full success, 10 when any task
/// failed or was skipped).
async fn run_batch(manifest_path: &str, _verbose: bool) -> AnyResult<u8> {
  use siwi_download::download::queue::{DownloadQueue, DownloadTask};

  let raw = std::fs::read_to_string(manifest_path)
    .map_err(|e| anyhow::anyhow!("cannot read batch manifest `{manifest_path}`: {e}"))?;
  #[derive(Deserialize)]
  struct Manifest {
    #[serde(default)]
    concurrent: Option<usize>,
    #[serde(default)]
    state_file: Option<String>,
    #[serde(default)]
    tasks: Vec<DownloadTask>,
  }
  let manifest: Manifest = toml::from_str(&raw)
    .map_err(|e| anyhow::anyhow!("cannot parse batch manifest `{manifest_path}`: {e}"))?;
  if manifest.tasks.is_empty() {
    return Err(anyhow::anyhow!("batch manifest has no [[tasks]]"));
  }

  let mut queue = DownloadQueue::new(manifest.concurrent.unwrap_or(1).max(1));
  if let Some(state_file) = manifest.state_file.as_ref() {
    queue = queue.state_file(state_file);
  }
  for task in manifest.tasks {
    queue.push(task);
  }

  // Persist the validated definitions so an interrupted run can be re-loaded.
  if let Some(state_file) = queue.state_file.clone() {
    queue.save_state(&state_file)?;
  }

  let output = "./";
  let downloader = Download::new(output);
  downloader.auto_create_storage_path().await?;
  let base_options = DownloadOptions::default();
  let results = queue.run(&downloader, &base_options).await?;

  let failed: Vec<_> = results.iter().filter(|r| !r.is_success()).collect();
  for r in &results {
    let status = r
      .report
      .download_status
      .as_ref()
      .map(|s| format!("{s:?}"))
      .unwrap_or_else(|| "Unknown".into());
    let size = r
      .report
      .file_size
      .map(|s| s.to_string())
      .unwrap_or_else(|| "-".into());
    let msg = r.report.msg.clone().unwrap_or_default();
    println!("{:<20} {:<10} {:>12}  {}", r.task_id, status, size, msg);
  }
  println!(
    "\n{} task(s): {} ok, {} failed/skipped",
    results.len(),
    results.len() - failed.len(),
    failed.len()
  );

  Ok(if failed.is_empty() { 0 } else { 10 })
}
