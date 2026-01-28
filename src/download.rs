//! Download functionality for siwi-download.
//!
//! This module provides the core download engine, including:
//! - [`Download`] - The main downloader struct
//! - [`DownloadOptions`] - Configuration options for downloads
//! - [`DownloadReport`] - Download result and metadata
//! - [`DownloadStatus`] - Possible download states

use crate::{
  error::AnyResult,
  utils::{create_dir_all, date, get_file_name_from_url, get_file_size, is_dir},
};
use chrono::{DateTime, Utc};
use indicatif::{ProgressBar, ProgressState, ProgressStyle};
use reqwest::header::CONTENT_LENGTH;

use reqwest::header::{HeaderMap, HeaderValue, RANGE};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fmt::Write;
use std::{borrow::Cow, path::Path};
use tokio::{
  fs,
  io::AsyncWriteExt,
  time::{Duration, sleep},
};
use tracing::info;

/// HTTP status code for successful partial content response.
const HTTP_PARTIAL_CONTENT: u16 = 206;
/// HTTP status code for range not satisfiable.
const HTTP_RANGE_NOT_SATISFIABLE: u16 = 416;
/// HTTP redirect threshold - codes >= 300 are redirects or errors.
const HTTP_REDIRECT_THRESHOLD: u16 = 300;
/// Maximum number of retry attempts for HEAD requests.
const MAX_HEAD_REQUEST_RETRIES: u16 = 5;
/// Delay in seconds between HEAD request retries.
const HEAD_REQUEST_RETRY_DELAY_SECS: u64 = 3;

/// Configuration options for a download operation.
///
/// This struct uses the builder pattern to allow fluent configuration
/// of download parameters.
///
/// # Example
///
/// ```rust,no_run
/// use siwi_download::download::DownloadOptions;
/// use reqwest::header::HeaderMap;
///
/// let mut options = DownloadOptions::default();
/// options
///     .set_file_name("custom_name.zip")
///     .set_proxy("http://proxy.example.com:8080")
///     .set_show_progress(true);
/// ```
#[derive(Debug, Default)]
pub struct DownloadOptions<'a> {
  pub maybe_file_name: Option<Cow<'a, str>>,
  pub maybe_proxy: Option<Cow<'a, str>>,
  pub maybe_headers: Option<HeaderMap>,
  pub show_progress: bool,
}

impl<'a> DownloadOptions<'a> {
  /// Creates a new [`DownloadOptions`] with default values.
  ///
  /// This is equivalent to [`DownloadOptions::default()`].
  ///
  /// # Returns
  ///
  /// A new `DownloadOptions` instance with all fields set to `None` or `false`.
  pub fn new() -> Self {
    Self {
      maybe_file_name: None,
      maybe_proxy: None,
      maybe_headers: None,
      show_progress: false,
    }
  }

  /// Sets the HTTP proxy for the download.
  ///
  /// # Arguments
  ///
  /// * `proxy` - The proxy URL (e.g., `http://127.0.0.1:7890`)
  ///
  /// # Returns
  ///
  /// A mutable reference to `self` for method chaining.
  pub fn set_proxy<S>(&mut self, proxy: S) -> &mut Self
  where
    S: Into<Cow<'a, str>>,
  {
    self.maybe_proxy = Some(proxy.into());
    self
  }

  /// Sets custom HTTP headers for the request.
  ///
  /// # Arguments
  ///
  /// * `headers` - A [`HeaderMap`] containing custom headers
  ///
  /// # Returns
  ///
  /// A mutable reference to `self` for method chaining.
  pub fn set_headers(&mut self, headers: HeaderMap) -> &mut Self {
    self.maybe_headers = Some(headers);
    self
  }

  /// Sets whether to display a progress bar during download.
  ///
  /// # Arguments
  ///
  /// * `show_progress` - Whether to show the progress bar
  ///
  /// # Returns
  ///
  /// A mutable reference to `self` for method chaining.
  pub fn set_show_progress(&mut self, show_progress: bool) -> &mut Self {
    self.show_progress = show_progress;
    self
  }

  /// Sets the custom filename for the downloaded file.
  ///
  /// If not set, the filename will be extracted from the URL.
  ///
  /// # Arguments
  ///
  /// * `file_name` - The custom filename to use
  ///
  /// # Returns
  ///
  /// A mutable reference to `self` for method chaining.
  pub fn set_file_name<S: Into<Cow<'a, str>>>(&mut self, file_name: S) -> &mut Self {
    self.maybe_file_name = Some(file_name.into());
    self
  }
}

/// A report containing the results and metadata of a download operation.
///
/// This struct is returned by [`Download::download()`] and contains all
/// information about the download, including status, timing, and file details.
///
/// # Serialization
///
/// `DownloadReport` implements [`Serialize`] and [`Deserialize`] from Serde,
/// making it suitable for logging or storage. The `headers` field is skipped
/// during serialization to avoid circular references.
#[derive(Debug, Deserialize, Serialize)]
pub struct DownloadReport<'a> {
  pub url: Cow<'a, str>,
  pub file_name: Cow<'a, str>,
  pub origin_file_name: Cow<'a, str>,
  pub storage_path: Cow<'a, str>,
  pub file_path: Cow<'a, str>,
  pub file_size: Option<u64>,
  pub range_from: Option<u64>,
  pub download_start_at: Option<DateTime<Utc>>,
  pub download_end_at: Option<DateTime<Utc>>,
  pub download_status: Option<DownloadStatus>,
  #[serde(skip)]
  pub headers: Option<HeaderMap>,
  pub head_status: Option<u16>,
  pub resp_status: Option<u16>,
  pub time_used: Option<i64>,
  pub msg: Option<Cow<'a, str>>,
}

impl<'a> DownloadReport<'a> {
  /// Creates a new [`DownloadReport`] with the given initial values.
  ///
  /// All fields except `download_status` (set to [`DownloadStatus::Error`])
  /// are initialized to `None`.
  ///
  /// # Arguments
  ///
  /// * `url` - The download URL
  /// * `file_name` - The name the file will be saved as
  /// * `origin_file_name` - The original filename from the URL
  /// * `storage_path` - The directory where the file will be saved
  /// * `file_path` - The full path to the file
  pub fn new<S: Into<Cow<'a, str>>>(
    url: S,
    file_name: S,
    origin_file_name: S,
    storage_path: S,
    file_path: S,
  ) -> Self {
    Self {
      url: url.into(),
      file_name: file_name.into(),
      origin_file_name: origin_file_name.into(),
      storage_path: storage_path.into(),
      file_path: file_path.into(),
      file_size: None,
      range_from: None,
      download_start_at: None,
      download_end_at: None,
      download_status: Some(DownloadStatus::Error),
      headers: None,
      head_status: None,
      resp_status: None,
      time_used: None,
      msg: None,
    }
  }

  /// Sets the total file size in bytes.
  pub fn set_file_size(&mut self, file_size: u64) -> &mut Self {
    self.file_size = Some(file_size);
    self
  }

  /// Sets the byte position from which to resume the download.
  pub fn set_range_from(&mut self, range_from: u64) -> &mut Self {
    self.range_from = Some(range_from);
    self
  }

  /// Sets the download start timestamp.
  pub fn set_download_start_at(&mut self) -> &mut Self {
    self.download_start_at = Some(date());
    self
  }

  /// Sets the download end timestamp.
  pub fn set_download_end_at(&mut self) -> &mut Self {
    self.download_end_at = Some(date());
    self
  }

  /// Sets the current download status.
  pub fn set_download_status(&mut self, status: DownloadStatus) -> &mut Self {
    self.download_status = Some(status);
    self
  }

  /// Sets the HTTP response headers from the server.
  pub fn set_headers(&mut self, headers: HeaderMap) -> &mut Self {
    self.headers = Some(headers);
    self
  }

  /// Sets the HTTP status code from the HEAD request.
  pub fn set_head_status(&mut self, head_status: u16) -> &mut Self {
    self.head_status = Some(head_status);
    self
  }

  /// Sets the HTTP status code from the download request.
  pub fn set_resp_status(&mut self, resp_status: u16) -> &mut Self {
    self.resp_status = Some(resp_status);
    self
  }

  /// Sets a status message for the download.
  pub fn set_msg<S: Into<Cow<'a, str>>>(&mut self, msg: S) -> &mut Self {
    self.msg = Some(msg.into());
    self
  }

  /// Calculates and sets the time used for the download.
  ///
  /// This method computes the difference between `download_end_at`
  /// and `download_start_at`, setting `time_used` to the result in seconds.
  pub fn gen_time_used(&mut self) -> &mut Self {
    let mut time_used: i64 = 0;
    if let Some(end) = self.download_end_at
      && let Some(start) = self.download_start_at
    {
      time_used = end.timestamp() - start.timestamp();
    }
    self.time_used = Some(time_used);
    self
  }

  /// Sends the download report to a remote server.
  ///
  /// This method POSTs the download report as JSON to the specified URL.
  ///
  /// # Arguments
  ///
  /// * `url` - The URL to send the report to
  /// * `headers` - HTTP headers to include in the request
  ///
  /// # Returns
  ///
  /// The server response body as a string.
  ///
  /// # Errors
  ///
  /// Returns an error if the HTTP request fails or the server returns an error status.
  pub async fn report(&self, url: &str, headers: HeaderMap) -> AnyResult<Cow<'a, str>> {
    let client = reqwest::Client::builder()
      .no_proxy()
      .default_headers(headers)
      .build()?;
    // Serialize self as JSON and send in request body
    let report_json = json!({
      "url": self.url.as_ref(),
      "file_name": self.file_name.as_ref(),
      "file_size": self.file_size,
      "download_status": self.download_status,
      "time_used": self.time_used,
      "msg": self.msg.as_ref().map(|s| s.as_ref())
    });
    let res = client
      .post(url)
      .json(&report_json)
      .send()
      .await?
      .text()
      .await?;

    Ok(Cow::Owned(res.to_string()))
  }
}

/// The possible states of a download operation.
///
/// This enum represents the lifecycle of a download, from creation
/// through completion, including error states.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub enum DownloadStatus {
  /// File is being created (no partial download exists)
  Create,
  /// Download is resuming or appending to existing file
  Append,
  /// Download completed successfully
  Complete,
  /// File already exists (for 416 HTTP status)
  Exists,
  /// Download encountered an error
  Error,
}

/// The main downloader struct.
///
/// This struct manages the download process, including storage path
/// management and the actual download operation.
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
pub struct Download<'a> {
  pub storage_path: Cow<'a, str>,
}

impl<'a> Download<'a> {
  /// Creates a new [`Download`] instance for the given storage path.
  ///
  /// # Arguments
  ///
  /// * `storage_path` - The directory where downloaded files will be saved
  ///
  /// # Returns
  ///
  /// A new `Download` instance.
  pub fn new<S: Into<Cow<'a, str>>>(storage_path: S) -> Self {
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
    if !is_dir(self.storage_path.as_ref())? {
      create_dir_all(self.storage_path.as_ref()).await?;
      info!("create storage_path {}", &self.storage_path);
    }
    Ok(())
  }

  /// Downloads a file from the given URL.
  ///
  /// This is the core method of the downloader. It handles:
  /// - Extracting filename from URL (or using custom name)
  /// - Creating storage directory if needed
  /// - Making HTTP HEAD request to get file size
  /// - Making GET request with range header for resuming
  /// - Writing data to file with progress tracking
  /// - Handling HTTP errors and status codes
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
  /// - The HEAD request fails
  /// - The file cannot be opened for writing
  /// - The network connection fails
  pub async fn download<S: AsRef<str> + Clone + 'a>(
    self,
    url: S,
    options: DownloadOptions<'a>,
  ) -> AnyResult<DownloadReport<'a>> {
    let origin_file_name = get_file_name_from_url(url.as_ref())?;
    let file_name = match options.maybe_file_name {
      Some(file_name) => file_name,
      None => origin_file_name.clone(),
    };

    let file_path = format!("{}/{}", self.storage_path.as_ref(), file_name);

    let mut report = DownloadReport::new(
      Cow::Owned(url.as_ref().to_string()),
      Cow::Owned(file_name.to_string()),
      Cow::Owned(origin_file_name.to_string()),
      Cow::Owned(self.storage_path.to_string()),
      Cow::Owned(file_path.to_string()),
    );

    let file_path = Path::new(file_path.as_str());
    let file_size = get_file_size(file_path)?;

    report
      .set_file_size(file_size)
      .set_range_from(file_size)
      .set_download_start_at();

    // Handle custom headers
    let mut headers = match options.maybe_headers {
      Some(headers) => headers,
      None => HeaderMap::new(),
    };

    let range = format!("bytes={}-", file_size);
    headers.insert(RANGE, HeaderValue::from_str(range.as_str())?);
    report.set_headers(headers.clone());
    // Build HTTP client with optional proxy
    let client = match options.maybe_proxy {
      Some(proxy) => reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(proxy.as_ref())?)
        .default_headers(headers)
        .build()?,
      None => reqwest::Client::builder()
        .no_proxy()
        .default_headers(headers)
        .build()?,
    };
    // Send HEAD request to get file size
    let mut resp = client.head(url.as_ref()).send().await?;
    let mut status = resp.status().as_u16();

    // HEAD request should return 200 OK, or 206/416 if server supports Range on HEAD
    // Some servers return 206 or 416 even for HEAD requests with Range header
    if status != HTTP_PARTIAL_CONTENT && status != HTTP_RANGE_NOT_SATISFIABLE && status != 200 {
      let mut this_time: u16 = 0;
      resp = loop {
        resp = client.head(url.as_ref()).send().await?;
        status = resp.status().as_u16();
        info!("try {} time head status is {}", this_time, status);
        if status == HTTP_PARTIAL_CONTENT
          || status == HTTP_RANGE_NOT_SATISFIABLE
          || status >= MAX_HEAD_REQUEST_RETRIES
        {
          break resp;
        }
        this_time += 1;
        sleep(Duration::from_secs(HEAD_REQUEST_RETRY_DELAY_SECS)).await;
      };
    }

    report.set_head_status(status);

    // Check for redirect or error status (>= 300 includes 3xx redirects and 4xx/5xx errors)
    if status >= HTTP_REDIRECT_THRESHOLD {
      if status == HTTP_RANGE_NOT_SATISFIABLE {
        report.set_download_status(DownloadStatus::Exists);
        report.set_download_end_at();
        return Ok(report);
      } else {
        report.set_download_status(DownloadStatus::Error);
        report.set_download_end_at();
        return Ok(report);
      }
    }

    let mut content_length = 0;
    if let Some(hv) = resp.headers().get(CONTENT_LENGTH)
      && let Ok(l) = hv.to_str()
    {
      info!("Content-Length: {}", l);
      if let Ok(l) = l.parse::<u64>() {
        content_length = l;
      }
    }

    let total = file_size + content_length;
    report.set_file_size(total);
    let pb = ProgressBar::new(total);
    if options.show_progress {
      pb.set_style(
        ProgressStyle::with_template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({eta})")
          .unwrap()
          .with_key("eta", |state: &ProgressState, w: &mut dyn Write| {
            let eta = state.eta().as_secs_f64();
            if eta.is_finite() {
              let _ = write!(w, "{:.1}s", eta);
            }
          })
          .progress_chars("#>-"),
      );
    }

    let mut resp = client.get(url.as_ref()).send().await?;
    let status = resp.status().as_u16();
    report.set_resp_status(status);

    // Check for redirect or error status in download response
    if status >= HTTP_REDIRECT_THRESHOLD {
      if status == HTTP_RANGE_NOT_SATISFIABLE {
        report.set_download_status(DownloadStatus::Exists);
        report.set_download_end_at();
        report.set_msg("file exists".to_owned());
        return Ok(report);
      } else {
        report.set_download_status(DownloadStatus::Error);
        report.set_download_end_at();
        report.set_msg("download resp status error".to_owned());
        return Ok(report);
      }
    }

    let mut dest = fs::OpenOptions::new()
      .create(true)
      .append(true)
      .open(&file_path)
      .await?;

    if file_size > 0 {
      report.set_download_status(DownloadStatus::Append);
      if options.show_progress {
        pb.set_position(file_size);
      }
    } else {
      report.set_download_status(DownloadStatus::Create);
    }

    while let Some(chunk) = resp.chunk().await? {
      report.set_download_status(DownloadStatus::Append);
      dest.write_all(&chunk).await?;
      if options.show_progress {
        pb.inc(chunk.len() as u64);
      }
    }

    // Flush to ensure all data is written to disk
    dest.flush().await?;

    report.set_download_status(DownloadStatus::Complete);
    report.set_download_end_at().gen_time_used();

    Ok(report)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_download_options_default() {
    let options = DownloadOptions::default();
    assert!(options.maybe_file_name.is_none());
    assert!(options.maybe_proxy.is_none());
    assert!(options.maybe_headers.is_none());
    assert!(!options.show_progress);
  }

  #[test]
  fn test_download_options_new() {
    let options = DownloadOptions::new();
    assert!(options.maybe_file_name.is_none());
    assert!(options.maybe_proxy.is_none());
    assert!(options.maybe_headers.is_none());
    assert!(!options.show_progress);
  }

  #[test]
  fn test_download_options_builder_pattern() {
    let mut options = DownloadOptions::new();
    options
      .set_file_name("test.txt")
      .set_proxy("http://proxy.example.com")
      .set_show_progress(true);

    assert_eq!(Some(Cow::Borrowed("test.txt")), options.maybe_file_name);
    assert_eq!(
      Some(Cow::Borrowed("http://proxy.example.com")),
      options.maybe_proxy
    );
    assert!(options.show_progress);
  }

  #[test]
  fn test_download_status_variants() {
    let _ = DownloadStatus::Create;
    let _ = DownloadStatus::Append;
    let _ = DownloadStatus::Complete;
    let _ = DownloadStatus::Exists;
    let _ = DownloadStatus::Error;
  }

  #[test]
  fn test_download_report_new() {
    let report = DownloadReport::new(
      "https://example.com/file.txt",
      "file.txt",
      "origin.txt",
      "/storage",
      "/storage/file.txt",
    );

    assert_eq!(report.url, "https://example.com/file.txt");
    assert_eq!(report.file_name, "file.txt");
    assert_eq!(report.origin_file_name, "origin.txt");
    assert_eq!(report.storage_path, "/storage");
    assert_eq!(report.file_path, "/storage/file.txt");
    assert!(report.file_size.is_none());
    assert_eq!(report.download_status, Some(DownloadStatus::Error));
  }

  #[test]
  fn test_download_report_builder_pattern() {
    let mut report = DownloadReport::new(
      "https://example.com/file.txt",
      "file.txt",
      "file.txt",
      "/storage",
      "/storage/file.txt",
    );

    report
      .set_file_size(1024)
      .set_range_from(512)
      .set_download_status(DownloadStatus::Complete)
      .set_head_status(200)
      .set_resp_status(200)
      .set_msg("success");

    assert_eq!(Some(1024), report.file_size);
    assert_eq!(Some(512), report.range_from);
    assert_eq!(Some(DownloadStatus::Complete), report.download_status);
    assert_eq!(Some(200), report.head_status);
    assert_eq!(Some(200), report.resp_status);
    assert_eq!(Some(Cow::Borrowed("success")), report.msg);
  }

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
  fn test_download_gen_time_used() {
    let mut report = DownloadReport::new(
      "https://example.com/file.txt",
      "file.txt",
      "file.txt",
      "/storage",
      "/storage/file.txt",
    );

    report
      .set_download_start_at()
      .set_download_end_at()
      .gen_time_used();

    // time_used should be 0 or positive (timestamps are very close)
    assert!(report.time_used.unwrap() >= 0);
  }
}
