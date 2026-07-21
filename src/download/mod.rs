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

pub mod client;
pub mod options;
pub mod report;

pub use options::DownloadOptions;
pub use report::{DownloadReport, DownloadStatus};

use crate::{
  error::AnyResult,
  utils::{create_dir_all, get_file_name_from_url, get_file_size, is_dir},
};
use indicatif::{ProgressBar, ProgressState, ProgressStyle};
use reqwest::header::CONTENT_LENGTH;
use reqwest::header::{HeaderMap, HeaderValue, RANGE};
use std::fmt::Write;
use std::path::Path;
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
/// HTTP redirect threshold - codes >= 300 are redirects or errors.
const HTTP_REDIRECT_THRESHOLD: u16 = 300;
/// Maximum number of retry attempts for HEAD requests.
const MAX_HEAD_REQUEST_RETRIES: u32 = 5;
/// Delay in seconds between HEAD request retries.
const HEAD_REQUEST_RETRY_DELAY_SECS: u64 = 3;

/// Returns `true` if `status` is one of the codes we treat as a successful
/// response to a HEAD (or GET) request: `200`, `206`, or `416`.
fn is_acceptable_status(status: u16) -> bool {
  matches!(
    status,
    HTTP_OK | HTTP_PARTIAL_CONTENT | HTTP_RANGE_NOT_SATISFIABLE
  )
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

    let file_path = format!("{}/{}", self.storage_path, file_name);

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
    report.set_headers(headers.clone());

    // Build HTTP client with timeouts and optional proxy.
    let client = client::build_client(options.maybe_proxy.as_deref(), headers)?;

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

    // Map the GET response status the same way as HEAD.
    if status >= HTTP_REDIRECT_THRESHOLD {
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

    while let Some(chunk) = resp.chunk().await? {
      dest.write_all(&chunk).await?;
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
      .gen_time_used();

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
    assert!(is_acceptable_status(416));
    assert!(!is_acceptable_status(404));
    assert!(!is_acceptable_status(500));
    assert!(!is_acceptable_status(301));
  }
}
