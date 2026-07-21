//! Configuration options for downloads.
//!
//! This module exposes [`DownloadOptions`], a builder-style configuration
//! struct consumed by [`Download::download`](super::Download::download).

use reqwest::header::HeaderMap;

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
pub struct DownloadOptions {
  /// Optional override for the downloaded file's name.
  pub maybe_file_name: Option<String>,
  /// Optional HTTP/HTTPS proxy URL.
  pub maybe_proxy: Option<String>,
  /// Optional custom HTTP headers merged into every request.
  pub maybe_headers: Option<HeaderMap>,
  /// Whether to render a progress bar during the download.
  pub show_progress: bool,
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
}
