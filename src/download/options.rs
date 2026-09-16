//! Configuration options for downloads.
//!
//! This module exposes [`DownloadOptions`], a builder-style configuration
//! struct consumed by [`Download::download`](super::Download::download).

use super::checksum::Algorithm;
use super::events::DownloadHook;
use chrono::{DateTime, Utc};
use reqwest::header::HeaderMap;
use std::fmt;
use std::sync::Arc;

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
#[derive(Default)]
pub struct DownloadOptions {
  /// Optional override for the downloaded file's name.
  pub maybe_file_name: Option<String>,
  /// Optional HTTP/HTTPS proxy URL.
  pub maybe_proxy: Option<String>,
  /// Optional custom HTTP headers merged into every request.
  pub maybe_headers: Option<HeaderMap>,
  /// Whether to render a progress bar during the download.
  pub show_progress: bool,
  /// Optional checksum the downloaded file must match
  /// `(algorithm, expected hex digest)`.
  pub maybe_checksum: Option<(Algorithm, String)>,
  /// Optional average-speed cap in bytes per second.
  pub max_speed: Option<u64>,
  /// Optional `If-Modified-Since` timestamp; a `304` response marks the
  /// report as `not_modified` and skips the body download.
  pub maybe_if_modified_since: Option<DateTime<Utc>>,
  /// Optional `If-None-Match` entity tag; a `304` response marks the
  /// report as `not_modified` and skips the body download.
  pub maybe_if_none_match: Option<String>,
  /// Hooks observing the download lifecycle, invoked in registration order.
  pub hooks: Vec<Arc<dyn DownloadHook>>,
}

impl fmt::Debug for DownloadOptions {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.debug_struct("DownloadOptions")
      .field("maybe_file_name", &self.maybe_file_name)
      .field("maybe_proxy", &self.maybe_proxy)
      .field("maybe_headers", &self.maybe_headers)
      .field("show_progress", &self.show_progress)
      .field("maybe_checksum", &self.maybe_checksum)
      .field("max_speed", &self.max_speed)
      .field("maybe_if_modified_since", &self.maybe_if_modified_since)
      .field("maybe_if_none_match", &self.maybe_if_none_match)
      .field("hooks", &self.hooks.len())
      .finish()
  }
}

impl DownloadOptions {
  /// Creates a new [`DownloadOptions`] with default values.
  ///
  /// This is equivalent to [`DownloadOptions::default()`].
  ///
  /// # Returns
  ///
  /// A new `DownloadOptions` instance with all fields set to `None` or `false`.
  #[must_use]
  pub fn new() -> Self {
    Self {
      maybe_file_name: None,
      maybe_proxy: None,
      maybe_headers: None,
      show_progress: false,
      maybe_checksum: None,
      max_speed: None,
      maybe_if_modified_since: None,
      maybe_if_none_match: None,
      hooks: Vec::new(),
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
  pub fn set_proxy<S: Into<String>>(&mut self, proxy: S) -> &mut Self {
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
  pub fn set_file_name<S: Into<String>>(&mut self, file_name: S) -> &mut Self {
    self.maybe_file_name = Some(file_name.into());
    self
  }

  /// Sets the checksum the downloaded file must match.
  ///
  /// After a successful download the file is hashed with `algo` and compared
  /// (case-insensitively) against `expected_hex`; the outcome is recorded in
  /// [`DownloadReport::checksum_verified`](super::DownloadReport). A mismatch
  /// fails the download (`DownloadStatus::Error`).
  ///
  /// # Arguments
  ///
  /// * `algo` - Hash algorithm to verify with
  /// * `expected_hex` - Expected digest as a hex string
  ///
  /// # Returns
  ///
  /// A mutable reference to `self` for method chaining.
  pub fn set_checksum(&mut self, algo: Algorithm, expected_hex: impl Into<String>) -> &mut Self {
    self.maybe_checksum = Some((algo, expected_hex.into()));
    self
  }

  /// Caps the average download speed at `bytes_per_sec`.
  ///
  /// The cap is enforced between chunks: if data arrives faster than the
  /// limit, the writer sleeps to keep the average at or below the rate.
  ///
  /// # Returns
  ///
  /// A mutable reference to `self` for method chaining.
  pub fn set_max_speed(&mut self, bytes_per_sec: u64) -> &mut Self {
    self.max_speed = Some(bytes_per_sec);
    self
  }

  /// Adds an `If-Modified-Since` precondition.
  ///
  /// When the server answers `304 Not Modified`, nothing is downloaded and
  /// [`DownloadReport::not_modified`](super::DownloadReport) is set to
  /// `Some(true)`.
  ///
  /// # Returns
  ///
  /// A mutable reference to `self` for method chaining.
  pub fn set_if_modified_since(&mut self, since: DateTime<Utc>) -> &mut Self {
    self.maybe_if_modified_since = Some(since);
    self
  }

  /// Adds an `If-None-Match` precondition with the given entity tag.
  ///
  /// When the server answers `304 Not Modified`, nothing is downloaded and
  /// [`DownloadReport::not_modified`](super::DownloadReport) is set to
  /// `Some(true)`.
  ///
  /// # Returns
  ///
  /// A mutable reference to `self` for method chaining.
  pub fn set_if_none_match<S: Into<String>>(&mut self, etag: S) -> &mut Self {
    self.maybe_if_none_match = Some(etag.into());
    self
  }

  /// Registers a lifecycle [`DownloadHook`].
  ///
  /// Hooks run in registration order and observe every
  /// [`DownloadEvent`](super::events::DownloadEvent). A hook returning an
  /// error aborts the download.
  ///
  /// # Returns
  ///
  /// A mutable reference to `self` for method chaining.
  pub fn add_hook(&mut self, hook: Arc<dyn DownloadHook>) -> &mut Self {
    self.hooks.push(hook);
    self
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

    assert_eq!(Some("test.txt".to_owned()), options.maybe_file_name);
    assert_eq!(
      Some("http://proxy.example.com".to_owned()),
      options.maybe_proxy
    );
    assert!(options.show_progress);
  }

  #[test]
  fn test_download_options_checksum() {
    let mut options = DownloadOptions::default();
    let expected = "a".repeat(64);
    options.set_checksum(Algorithm::Sha256, expected.clone());
    assert_eq!(Some((Algorithm::Sha256, expected)), options.maybe_checksum);
  }

  #[test]
  fn test_download_options_max_speed() {
    let mut options = DownloadOptions::default();
    options.set_max_speed(1024 * 1024);
    assert_eq!(Some(1024 * 1024), options.max_speed);
  }

  #[test]
  fn test_download_options_conditional() {
    let mut options = DownloadOptions::default();
    let since = Utc::now();
    options
      .set_if_modified_since(since)
      .set_if_none_match("\"abc123\"");

    assert_eq!(Some(since), options.maybe_if_modified_since);
    assert_eq!(Some("\"abc123\"".to_owned()), options.maybe_if_none_match);
  }

  #[test]
  fn test_download_options_new_fields_default_none() {
    let options = DownloadOptions::default();
    assert!(options.maybe_checksum.is_none());
    assert!(options.max_speed.is_none());
    assert!(options.maybe_if_modified_since.is_none());
    assert!(options.maybe_if_none_match.is_none());
    assert!(options.hooks.is_empty());
  }

  #[test]
  fn test_download_options_add_hook() {
    use crate::download::events::RecordingHook;

    let mut options = DownloadOptions::default();
    options
      .add_hook(Arc::new(RecordingHook::new()))
      .add_hook(Arc::new(RecordingHook::new()));
    assert_eq!(2, options.hooks.len());
  }

  #[test]
  fn test_download_options_debug_lists_hook_count() {
    use crate::download::events::RecordingHook;

    let mut options = DownloadOptions::default();
    options.add_hook(Arc::new(RecordingHook::new()));
    let debug = format!("{options:?}");
    assert!(debug.contains("hooks: 1"), "{debug}");
    // Hook internals never leak through Debug.
    assert!(!debug.contains("RecordingHook"));
  }
}
