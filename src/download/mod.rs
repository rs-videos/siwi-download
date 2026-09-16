//! Download functionality for siwi-download.
//!
//! The module is organized as follows:
//! - [`Download`] - The main downloader struct (this file)
//! - [`DownloadOptions`] - Configuration options ([`options`])
//! - [`DownloadReport`] / [`DownloadStatus`] - Result types ([`report`])
//! - [`client`] - HTTP client construction helpers
//!
//! # Example
//!
//! ```rust,no_run
//! use siwi_download::download::{Download, DownloadOptions};
//!
//! #[tokio::main]
//! async fn main() -> siwi_download::error::AnyResult<()> {
//!     let download = Download::new("./downloads");
//!     download.auto_create_storage_path().await?;
//!
//!     let mut options = DownloadOptions::default();
//!     options.set_show_progress(true);
//!
//!     let report = download
//!         .download("https://example.com/file.zip", options)
//!         .await?;
//!
//!     println!("Download completed: {:?}", report);
//!     Ok(())
//! }
//! ```

pub mod checksum;
pub mod client;
pub mod events;
pub mod options;
pub mod report;
pub mod sink;

pub use checksum::Algorithm;
pub use events::{CommandHook, DownloadEvent, DownloadHook, FailingHook, LogHook, RecordingHook};
pub use options::DownloadOptions;
pub use report::{DownloadReport, DownloadStatus};
pub use sink::StreamSink;

use crate::{
  error::AnyResult,
  utils::{create_dir_all, get_file_name_from_url, get_file_size, is_dir},
};
use chrono::{DateTime, Utc};
use indicatif::{ProgressBar, ProgressState, ProgressStyle};
use reqwest::header::CONTENT_LENGTH;
use reqwest::header::{HeaderMap, HeaderValue, IF_MODIFIED_SINCE, IF_NONE_MATCH, RANGE};
use std::fmt::Write;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tokio::{
  fs,
  io::AsyncWriteExt,
  time::{Duration, sleep},
};
use tracing::{info, warn};

/// HTTP status code for successful OK response.
const HTTP_OK: u16 = 200;
/// HTTP status code for successful partial content response.
const HTTP_PARTIAL_CONTENT: u16 = 206;
/// HTTP status code for range not satisfiable.
const HTTP_RANGE_NOT_SATISFIABLE: u16 = 416;
/// HTTP status code for "not modified" answers to conditional requests.
const HTTP_NOT_MODIFIED: u16 = 304;
/// HTTP redirect threshold - codes >= 300 are redirects or errors.
const HTTP_REDIRECT_THRESHOLD: u16 = 300;
/// Maximum number of retry attempts for HEAD requests.
const MAX_HEAD_REQUEST_RETRIES: u32 = 5;
/// Delay in seconds between HEAD request retries.
const HEAD_REQUEST_RETRY_DELAY_SECS: u64 = 3;

/// Returns `true` if `status` is one of the codes we treat as a successful
/// response to a HEAD (or GET) request: `200`, `206`, `304`, or `416`.
///
/// `304` counts as acceptable because it is a valid (terminal) answer to a
/// conditional request, not a transient failure worth retrying.
fn is_acceptable_status(status: u16) -> bool {
  matches!(
    status,
    HTTP_OK | HTTP_PARTIAL_CONTENT | HTTP_NOT_MODIFIED | HTTP_RANGE_NOT_SATISFIABLE
  )
}

/// Formats a timestamp as an HTTP-date (IMF-fixdate, RFC 9110 §5.6.7),
/// e.g. `Sun, 06 Nov 1994 08:49:37 GMT`.
fn format_http_date(dt: DateTime<Utc>) -> String {
  dt.format("%a, %d %b %Y %H:%M:%S GMT").to_string()
}

/// Delivers `event` to every registered hook, in registration order.
///
/// The first hook that returns an error aborts the download: the error is
/// propagated to the caller and no further hooks (or events) run.
fn dispatch_hooks(hooks: &[Arc<dyn DownloadHook>], event: &DownloadEvent<'_>) -> AnyResult<()> {
  for hook in hooks {
    hook.on_event(event.clone())?;
  }
  Ok(())
}

/// The main downloader struct.
///
/// This struct manages the download process, including storage path
/// management and the actual download operation. A single instance can
/// be reused across multiple [`Download::download`] calls.
///
/// # Example
///
/// ```rust,no_run
/// use siwi_download::download::{Download, DownloadOptions};
///
/// #[tokio::main]
/// async fn main() -> siwi_download::error::AnyResult<()> {
///     let download = Download::new("./downloads");
///     download.auto_create_storage_path().await?;
///
///     let mut options = DownloadOptions::default();
///     options.set_show_progress(true);
///
///     let report = download
///         .download("https://example.com/file.zip", options)
///         .await?;
///
///     println!("Download completed: {:?}", report);
///     Ok(())
/// }
/// ```
pub struct Download {
  /// Directory the downloaded files are written to.
  pub storage_path: String,
}

impl Download {
  /// Creates a new [`Download`] instance for the given storage path.
  ///
  /// # Arguments
  ///
  /// * `storage_path` - The directory where downloaded files will be saved
  ///
  /// # Returns
  ///
  /// A new `Download` instance.
  #[must_use]
  pub fn new<S: Into<String>>(storage_path: S) -> Self {
    Self {
      storage_path: storage_path.into(),
    }
  }

  /// Ensures the storage directory exists.
  ///
  /// This method checks if the storage path exists as a directory,
  /// and creates it if it doesn't.
  ///
  /// # Returns
  ///
  /// `Ok(())` if the directory exists or was created successfully.
  ///
  /// # Errors
  ///
  /// Returns an error if the directory cannot be created due to permission
  /// denied, disk full, or other I/O errors.
  pub async fn auto_create_storage_path(&self) -> AnyResult<()> {
    if !is_dir(&self.storage_path).await {
      create_dir_all(&self.storage_path).await?;
      info!("create storage_path {}", &self.storage_path);
    }
    Ok(())
  }

  /// Downloads a file from the given URL.
  ///
  /// This is the core method of the downloader. It handles:
  /// - Extracting filename from URL (or using a custom name)
  /// - Creating the storage directory if needed
  /// - Making an HTTP HEAD request (with bounded retry) to probe the server
  /// - Making a GET request with a `Range` header for resuming
  /// - Writing data to file with optional progress tracking
  /// - Mapping HTTP status codes to [`DownloadStatus`]
  ///
  /// Takes `&self` so the same [`Download`] can be reused for multiple files.
  ///
  /// # Arguments
  ///
  /// * `url` - The URL of the file to download
  /// * `options` - Configuration options for the download
  ///
  /// # Returns
  ///
  /// A [`DownloadReport`] containing download metadata and status.
  ///
  /// # Errors
  ///
  /// This method returns an error if:
  /// - The URL is invalid
  /// - The HEAD request fails after all retries
  /// - The file cannot be opened for writing
  /// - The network connection fails
  #[allow(clippy::too_many_lines)]
  pub async fn download(
    &self,
    url: impl AsRef<str>,
    options: DownloadOptions,
  ) -> AnyResult<DownloadReport> {
    let url_ref = url.as_ref();
    let origin_file_name = get_file_name_from_url(url_ref)?;
    let file_name = options
      .maybe_file_name
      .clone()
      .unwrap_or_else(|| origin_file_name.clone());

    // Native separators (Path::join, not string concat) so the reported path
    // is valid on every platform, Windows included.
    let file_path_buf = Path::new(self.storage_path.as_str()).join(&file_name);
    let file_path = file_path_buf.to_string_lossy().into_owned();

    let mut report = DownloadReport::new(
      url_ref.to_owned(),
      file_name,
      origin_file_name,
      self.storage_path.clone(),
      file_path.clone(),
    );

    let file_path = Path::new(&file_path);
    // Bytes already present on disk from a previous (interrupted) run.
    let local_size = get_file_size(file_path).await;

    report.set_range_from(local_size).set_download_start_at();

    // Build the Range header for resuming.
    let mut headers = match options.maybe_headers {
      Some(headers) => headers,
      None => HeaderMap::new(),
    };
    let range = format!("bytes={local_size}-");
    headers.insert(RANGE, HeaderValue::from_str(&range)?);

    // Conditional-request headers: when the caller already knows the remote
    // state, a `304 Not Modified` answer skips the body download entirely.
    if let Some(since) = options.maybe_if_modified_since {
      let value = format_http_date(since);
      headers.insert(IF_MODIFIED_SINCE, HeaderValue::from_str(&value)?);
    }
    if let Some(etag) = options.maybe_if_none_match.as_ref() {
      headers.insert(IF_NONE_MATCH, HeaderValue::from_str(etag)?);
    }
    report.set_headers(headers.clone());

    // Build HTTP client with timeouts and optional proxy.
    let client = client::build_client(options.maybe_proxy.as_deref(), headers)?;

    dispatch_hooks(
      &options.hooks,
      &DownloadEvent::BeforeRequest { url: url_ref },
    )?;

    // HEAD request to probe the server, with bounded retry on transient
    // non-acceptable responses. We only break early on an acceptable status;
    // otherwise we sleep and retry up to MAX_HEAD_REQUEST_RETRIES times.
    let mut resp = client.head(url_ref).send().await?;
    let mut status = resp.status().as_u16();
    let mut attempt: u32 = 0;
    while !is_acceptable_status(status) && attempt < MAX_HEAD_REQUEST_RETRIES {
      attempt += 1;
      warn!(
        attempt,
        status, "HEAD request returned non-acceptable status, retrying"
      );
      sleep(Duration::from_secs(HEAD_REQUEST_RETRY_DELAY_SECS)).await;
      resp = client.head(url_ref).send().await?;
      status = resp.status().as_u16();
    }
    report.set_head_status(status);

    // After retries, map redirect/error codes to terminal states.
    if status >= HTTP_REDIRECT_THRESHOLD {
      if status == HTTP_NOT_MODIFIED {
        // The server honored our conditional request: local copy is current.
        report
          .set_not_modified()
          .set_download_status(DownloadStatus::Exists)
          .set_download_end_at()
          .gen_time_used()
          .set_msg("not modified");
        return Ok(report);
      }
      if status == HTTP_RANGE_NOT_SATISFIABLE {
        report
          .set_download_status(DownloadStatus::Exists)
          .set_download_end_at()
          .gen_time_used()
          .set_msg("file exists");
        return Ok(report);
      }
      report
        .set_download_status(DownloadStatus::Error)
        .set_download_end_at()
        .gen_time_used()
        .set_msg(format!("head resp status error: {status}"));
      let msg = report.msg.clone().unwrap_or_default();
      dispatch_hooks(&options.hooks, &DownloadEvent::Error { msg: &msg })?;
      return Ok(report);
    }

    // Use the Content-Length from HEAD to compute the total size for the
    // progress bar. For a 206 response Content-Length is the remaining bytes;
    // for 200 the server ignored our Range and will resend everything.
    let mut content_length: u64 = 0;
    if let Some(hv) = resp.headers().get(CONTENT_LENGTH)
      && let Ok(l) = hv.to_str()
      && let Ok(l) = l.parse::<u64>()
    {
      info!("Content-Length: {}", l);
      content_length = l;
    }

    dispatch_hooks(
      &options.hooks,
      &DownloadEvent::HeadersReceived {
        url: url_ref,
        status,
        content_length: (content_length > 0).then_some(content_length),
      },
    )?;

    let total = if status == HTTP_PARTIAL_CONTENT {
      // Server honored the range: on-disk bytes + remaining bytes.
      local_size + content_length
    } else {
      // Server ignored the range (full response); restart from byte 0.
      content_length
    };
    report.set_file_size(total);

    // Only construct the (relatively costly) progress bar when requested.
    let pb = if options.show_progress {
      Some(build_progress_bar(
        total,
        status == HTTP_PARTIAL_CONTENT,
        local_size,
      ))
    } else {
      None
    };

    let mut resp = client.get(url_ref).send().await?;
    let status = resp.status().as_u16();
    report.set_resp_status(status);

    // Map the GET response status the same way as HEAD. 304 must be handled
    // before opening the destination file — opening in append mode would
    // touch its mtime and defeat the next `If-Modified-Since` check.
    if status >= HTTP_REDIRECT_THRESHOLD {
      if status == HTTP_NOT_MODIFIED {
        report
          .set_not_modified()
          .set_download_status(DownloadStatus::Exists)
          .set_download_end_at()
          .gen_time_used()
          .set_msg("not modified");
        return Ok(report);
      }
      if status == HTTP_RANGE_NOT_SATISFIABLE {
        report
          .set_download_status(DownloadStatus::Exists)
          .set_download_end_at()
          .gen_time_used()
          .set_msg("file exists");
        return Ok(report);
      }
      report
        .set_download_status(DownloadStatus::Error)
        .set_download_end_at()
        .gen_time_used()
        .set_msg(format!("download resp status error: {status}"));
      let msg = report.msg.clone().unwrap_or_default();
      dispatch_hooks(&options.hooks, &DownloadEvent::Error { msg: &msg })?;
      return Ok(report);
    }

    let mut dest = fs::OpenOptions::new()
      .create(true)
      .append(true)
      .open(file_path)
      .await?;

    // Set the initial lifecycle state once; the chunk loop below only advances
    // it to Complete on success.
    if local_size > 0 {
      report.set_download_status(DownloadStatus::Append);
    } else {
      report.set_download_status(DownloadStatus::Create);
    }

    let download_started = Instant::now();
    let mut last_progress: Option<Instant> = None;
    let mut written_since_start: u64 = 0;
    while let Some(chunk) = resp.chunk().await? {
      let offset = local_size + written_since_start;
      dest.write_all(&chunk).await?;
      written_since_start += chunk.len() as u64;

      dispatch_hooks(
        &options.hooks,
        &DownloadEvent::ChunkWritten {
          offset,
          len: chunk.len(),
        },
      )?;

      // Coarse progress events, rate-limited so slow hooks and log spam
      // cannot fire once per chunk.
      let now = Instant::now();
      let due = last_progress.is_none_or(|t| now.duration_since(t) >= events::PROGRESS_INTERVAL);
      if due {
        dispatch_hooks(
          &options.hooks,
          &DownloadEvent::Progress {
            downloaded: local_size + written_since_start,
            total: Some(total),
          },
        )?;
        last_progress = Some(now);
      }

      // Average-speed cap: if we are ahead of the allowed pace, sleep the
      // difference so the average never exceeds `max_speed`.
      if let Some(max_speed) = options.max_speed
        && max_speed > 0
      {
        // f64 is exact for byte counts below 2^53 — far beyond any real
        // download — so the precision-loss casts are safe here.
        #[allow(clippy::cast_precision_loss)]
        let allowed_elapsed = written_since_start as f64 / max_speed as f64;
        let ahead_secs = allowed_elapsed - download_started.elapsed().as_secs_f64();
        if ahead_secs > 0.0 {
          sleep(Duration::from_secs_f64(ahead_secs)).await;
        }
      }
      if let Some(pb) = pb.as_ref() {
        pb.inc(chunk.len() as u64);
      }
    }

    // Flush to ensure all data is written to disk.
    dest.flush().await?;
    if let Some(pb) = pb {
      pb.finish();
    }

    report
      .set_download_status(DownloadStatus::Complete)
      .set_download_end_at()
      .gen_time_used()
      .gen_average_speed();

    // Verify the downloaded file against the caller-provided checksum.
    // The hash covers the whole on-disk file, so resumed downloads are
    // verified in full, not just the appended tail.
    if let Some((algo, expected)) = options.maybe_checksum.as_ref() {
      let ok = checksum::verify(file_path, *algo, expected).await?;
      report.set_checksum_verified(ok);
      if !ok {
        report
          .set_download_status(DownloadStatus::Error)
          .set_msg(format!(
            "checksum mismatch: expected {} {}",
            algo.name(),
            expected
          ));
        let msg = report.msg.clone().unwrap_or_default();
        dispatch_hooks(&options.hooks, &DownloadEvent::Error { msg: &msg })?;
        return Ok(report);
      }
      info!("checksum verified ({})", algo.name());
    }

    dispatch_hooks(&options.hooks, &DownloadEvent::Complete { report: &report })?;

    Ok(report)
  }

  /// Streams a download into an arbitrary [`StreamSink`] — no local file is
  /// created.
  ///
  /// Unlike [`Download::download`], this method has no resume semantics
  /// (there is no on-disk state to resume against): the request goes out
  /// without a `Range` header and the body is handed to `sink` chunk by
  /// chunk, then [`StreamSink::finalize`] is called. `options.maybe_checksum`
  /// is ignored here; use a [`HashSink`](sink::HashSink) to hash in-flight
  /// instead. The report's `file_path` is empty and `range_from` is `0`.
  ///
  /// Hooks and rate limiting work exactly as in [`Download::download`]; in
  /// `ChunkWritten` events the `offset` counts bytes handed to the sink.
  ///
  /// Takes `&self` so the same [`Download`] can be reused.
  ///
  /// # Arguments
  ///
  /// * `url` - The URL of the file to download
  /// * `options` - Configuration options (proxy, headers, hooks, max speed…)
  /// * `sink` - Receives the body chunks; kept by the caller so sinks like
  ///   [`HashSink`](sink::HashSink) can be inspected afterwards
  ///
  /// # Returns
  ///
  /// A [`DownloadReport`] with the byte count in `file_size`.
  ///
  /// # Errors
  ///
  /// Returns an error if any request fails or the sink rejects a chunk.
  #[allow(clippy::too_many_lines)]
  pub async fn stream<S>(
    &self,
    url: impl AsRef<str>,
    options: DownloadOptions,
    sink: &mut S,
  ) -> AnyResult<DownloadReport>
  where
    S: StreamSink,
  {
    let url_ref = url.as_ref();
    let origin_file_name = get_file_name_from_url(url_ref)?;
    let file_name = options
      .maybe_file_name
      .clone()
      .unwrap_or_else(|| origin_file_name.clone());

    let mut report = DownloadReport::new(
      url_ref.to_owned(),
      file_name,
      origin_file_name,
      self.storage_path.clone(),
      String::new(), // no file on disk
    );

    report.set_range_from(0).set_download_start_at();

    let headers = match options.maybe_headers {
      Some(headers) => headers,
      None => HeaderMap::new(),
    };
    report.set_headers(headers.clone());

    let client = client::build_client(options.maybe_proxy.as_deref(), headers)?;

    dispatch_hooks(
      &options.hooks,
      &DownloadEvent::BeforeRequest { url: url_ref },
    )?;

    // No Range header here: streaming always starts from byte 0.
    let mut resp = client.get(url_ref).send().await?;
    let status = resp.status().as_u16();
    report.set_resp_status(status);

    if status >= HTTP_REDIRECT_THRESHOLD {
      report
        .set_download_status(DownloadStatus::Error)
        .set_download_end_at()
        .gen_time_used()
        .set_msg(format!("download resp status error: {status}"));
      let msg = report.msg.clone().unwrap_or_default();
      dispatch_hooks(&options.hooks, &DownloadEvent::Error { msg: &msg })?;
      return Ok(report);
    }

    let total = resp
      .headers()
      .get(CONTENT_LENGTH)
      .and_then(|hv| hv.to_str().ok())
      .and_then(|l| l.parse::<u64>().ok());

    dispatch_hooks(
      &options.hooks,
      &DownloadEvent::HeadersReceived {
        url: url_ref,
        status,
        content_length: total,
      },
    )?;

    report.set_file_size(0);

    // Only construct the (relatively costly) progress bar when requested.
    let pb = if options.show_progress {
      Some(build_progress_bar(total.unwrap_or(0), false, 0))
    } else {
      None
    };

    let download_started = Instant::now();
    let mut last_progress: Option<Instant> = None;
    let mut processed: u64 = 0;
    while let Some(chunk) = resp.chunk().await? {
      sink.write_chunk(&chunk).await?;
      processed += chunk.len() as u64;

      dispatch_hooks(
        &options.hooks,
        &DownloadEvent::ChunkWritten {
          offset: processed - chunk.len() as u64,
          len: chunk.len(),
        },
      )?;

      let now = Instant::now();
      let due = last_progress.is_none_or(|t| now.duration_since(t) >= events::PROGRESS_INTERVAL);
      if due {
        dispatch_hooks(
          &options.hooks,
          &DownloadEvent::Progress {
            downloaded: processed,
            total,
          },
        )?;
        last_progress = Some(now);
      }

      if let Some(max_speed) = options.max_speed
        && max_speed > 0
      {
        // f64 is exact for byte counts below 2^53 — far beyond any real
        // download — so the precision-loss casts are safe here.
        #[allow(clippy::cast_precision_loss)]
        let allowed_elapsed = processed as f64 / max_speed as f64;
        let ahead_secs = allowed_elapsed - download_started.elapsed().as_secs_f64();
        if ahead_secs > 0.0 {
          sleep(Duration::from_secs_f64(ahead_secs)).await;
        }
      }
      if let Some(pb) = pb.as_ref() {
        pb.inc(chunk.len() as u64);
      }
    }

    sink.finalize().await?;
    if let Some(pb) = pb {
      pb.finish();
    }

    report
      .set_file_size(processed)
      .set_download_status(DownloadStatus::Complete)
      .set_download_end_at()
      .gen_time_used()
      .gen_average_speed();

    dispatch_hooks(&options.hooks, &DownloadEvent::Complete { report: &report })?;

    Ok(report)
  }
}

/// Builds the progress bar used during download, styled and pre-positioned.
///
/// When `partial` is `true` and `local_size > 0`, the bar is advanced to the
/// on-disk position so the display reflects what's already been written.
fn build_progress_bar(total: u64, partial: bool, local_size: u64) -> ProgressBar {
  let pb = ProgressBar::new(total);
  pb.set_style(
    ProgressStyle::with_template(
      "{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({eta})",
    )
    .unwrap()
    .with_key("eta", |state: &ProgressState, w: &mut dyn Write| {
      let eta = state.eta().as_secs_f64();
      if eta.is_finite() {
        let _ = write!(w, "{eta:.1}s");
      }
    })
    .progress_chars("#>-"),
  );
  if partial && local_size > 0 {
    pb.set_position(local_size);
  }
  pb
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_download_new() {
    let download = Download::new("/storage");
    assert_eq!(download.storage_path, "/storage");
  }

  #[test]
  fn test_download_new_with_string() {
    let download = Download::new(String::from("/custom/path"));
    assert_eq!(download.storage_path, "/custom/path");
  }

  #[test]
  fn test_is_acceptable_status() {
    assert!(is_acceptable_status(200));
    assert!(is_acceptable_status(206));
    assert!(is_acceptable_status(304));
    assert!(is_acceptable_status(416));
    assert!(!is_acceptable_status(404));
    assert!(!is_acceptable_status(500));
    assert!(!is_acceptable_status(301));
  }

  #[test]
  fn test_format_http_date() {
    use chrono::TimeZone;
    // 1994-11-06 08:49:37 UTC — the RFC example timestamp.
    let dt = Utc
      .with_ymd_and_hms(1994, 11, 6, 8, 49, 37)
      .single()
      .unwrap();
    assert_eq!("Sun, 06 Nov 1994 08:49:37 GMT", format_http_date(dt));
  }
}
