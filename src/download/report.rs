//! Download result types.
//!
//! This module contains:
//! - [`DownloadReport`] - Result and metadata of a download operation
//! - [`DownloadStatus`] - Possible states a download can be in

use crate::{error::AnyResult, utils::date};
use chrono::{DateTime, Utc};
use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};

/// The possible states of a download operation.
///
/// This enum represents the lifecycle of a download, from creation
/// through completion, including error states.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone, Copy)]
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

/// A report containing the results and metadata of a download operation.
///
/// This struct is returned by [`Download::download`](super::Download::download)
/// and contains all information about the download, including status, timing,
/// and file details.
///
/// # Serialization
///
/// `DownloadReport` implements [`Serialize`] and [`Deserialize`] from Serde,
/// making it suitable for logging or storage. The `headers` field is skipped
/// during serialization to avoid leaking large header maps.
#[derive(Debug, Deserialize, Serialize)]
pub struct DownloadReport {
  /// The URL the file was downloaded from.
  pub url: String,
  /// The name the file was saved under (after any rename).
  pub file_name: String,
  /// The filename originally extracted from the URL.
  pub origin_file_name: String,
  /// The directory the file was written into.
  pub storage_path: String,
  /// The full path of the downloaded file on disk.
  pub file_path: String,
  /// Total size of the downloaded file in bytes (final on-disk size).
  pub file_size: Option<u64>,
  /// Byte offset the download resumed from (0 for a fresh download).
  pub range_from: Option<u64>,
  /// Timestamp the download started.
  pub download_start_at: Option<DateTime<Utc>>,
  /// Timestamp the download ended.
  pub download_end_at: Option<DateTime<Utc>>,
  /// Final status of the download.
  pub download_status: Option<DownloadStatus>,
  /// HTTP response headers captured from the server (skipped during serde).
  #[serde(skip)]
  pub headers: Option<HeaderMap>,
  /// HTTP status code returned by the HEAD request.
  pub head_status: Option<u16>,
  /// HTTP status code returned by the GET request.
  pub resp_status: Option<u16>,
  /// Wall-clock duration of the download in seconds.
  pub time_used: Option<i64>,
  /// Human-readable status message.
  pub msg: Option<String>,
}

impl DownloadReport {
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
  pub fn new(
    url: impl Into<String>,
    file_name: impl Into<String>,
    origin_file_name: impl Into<String>,
    storage_path: impl Into<String>,
    file_path: impl Into<String>,
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
  pub fn set_msg<S: Into<String>>(&mut self, msg: S) -> &mut Self {
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
  /// The report is serialized via its [`Serialize`] impl and `POST`ed as JSON
  /// in the request body, so the wire format stays in sync with the struct
  /// definition.
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
  /// Returns an error if serialization or the HTTP request fails.
  pub async fn report(&self, url: &str, headers: HeaderMap) -> AnyResult<String> {
    let client = reqwest::Client::builder()
      .no_proxy()
      .default_headers(headers)
      .build()?;
    let res = client.post(url).json(self).send().await?.text().await?;
    Ok(res)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

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
    assert_eq!(Some("success".to_owned()), report.msg);
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
