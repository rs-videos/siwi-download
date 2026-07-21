//! Utility functions for URL parsing, file operations, and timestamp handling.
//!
//! This module provides helper functions used throughout siwi-download
//! for extracting filenames from URLs, checking file/directory status,
//! and managing file system operations.

use crate::error::AnyResult;
use chrono::{DateTime, Utc};
use reqwest::Url;
use std::path::Path;
use tokio::fs;

/// Timestamp format used by [`gen_file_name`].
///
/// Uses only filename-safe characters (digits and `T`) to stay portable
/// across platforms, including Windows where `:` is forbidden in paths.
const FILE_NAME_TIMESTAMP_FMT: &str = "%Y%m%dT%H%M%S";

/// Extracts the filename from a URL.
///
/// This function parses the URL and returns the last path segment,
/// which is typically the filename.
///
/// # Arguments
///
/// * `url` - A string slice that holds the URL.
///
/// # Returns
///
/// Returns the filename, or an empty string if the URL has no path segments.
///
/// # Errors
///
/// Returns an error if the URL cannot be parsed.
pub(crate) fn get_file_name_from_url<S: AsRef<str>>(url: S) -> AnyResult<String> {
  let parse = Url::parse(url.as_ref())?;
  let file_name = parse.path_segments().and_then(Iterator::last).unwrap_or("");
  Ok(file_name.to_owned())
}

/// Returns the current UTC date and time.
///
/// This is a convenience function that wraps [`chrono::Utc::now()`]
/// for consistent timestamp generation throughout the library.
///
/// # Returns
///
/// Returns a [`DateTime<Utc>`] representing the current moment in UTC.
pub fn date() -> DateTime<Utc> {
  Utc::now()
}

/// Generates a unique filename with a timestamp prefix.
///
/// This function creates a new filename by prepending the current
/// timestamp to the original filename, useful for avoiding conflicts
/// when downloading files with the same name.
///
/// The timestamp uses only filename-safe characters (`YYYYmmddTHHMMSS`)
/// so the result is portable across platforms.
///
/// # Arguments
///
/// * `file_name` - The original filename.
///
/// # Returns
///
/// Returns the new filename in the format `{timestamp}_{original_filename}`.
///
/// # Example
///
/// ```rust
/// use siwi_download::utils::gen_file_name;
///
/// let new_name = gen_file_name("document.pdf");
/// assert!(new_name.ends_with("document.pdf"));
/// ```
pub fn gen_file_name<S: AsRef<str>>(file_name: S) -> String {
  let now = date();
  let ts = now.format(FILE_NAME_TIMESTAMP_FMT).to_string();
  format!("{}_{}", ts, file_name.as_ref())
}

/// Asynchronously creates all directories in the given path.
///
/// This is a wrapper around [`tokio::fs::create_dir_all`] that returns
/// a [`Result`] for better error handling.
///
/// # Arguments
///
/// * `src` - The path where directories should be created.
///
/// # Returns
///
/// Returns `Ok(())` on success, or an error if directory creation fails.
///
/// # Errors
///
/// Returns an error if the directory cannot be created.
pub async fn create_dir_all<S: AsRef<Path>>(src: S) -> AnyResult<()> {
  fs::create_dir_all(src.as_ref()).await?;
  Ok(())
}

/// Asynchronously checks if the given path points to a file.
///
/// This is a soft check: it returns `false` (rather than an error) if the
/// path doesn't exist or its metadata cannot be read. Use [`tokio::fs::metadata`]
/// directly if you need to distinguish "missing" from "permission denied".
///
/// # Arguments
///
/// * `dest` - The path to check.
///
/// # Returns
///
/// Returns `true` if the path exists and is a file, `false` otherwise.
pub async fn is_file<S: AsRef<Path>>(dest: S) -> bool {
  match fs::metadata(dest.as_ref()).await {
    Ok(metadata) => metadata.is_file(),
    Err(_) => false,
  }
}

/// Asynchronously checks if the given path points to a directory.
///
/// This is a soft check: it returns `false` (rather than an error) if the
/// path doesn't exist or its metadata cannot be read.
///
/// # Arguments
///
/// * `dest` - The path to check.
///
/// # Returns
///
/// Returns `true` if the path exists and is a directory, `false` otherwise.
pub async fn is_dir<S: AsRef<Path>>(dest: S) -> bool {
  match fs::metadata(dest.as_ref()).await {
    Ok(metadata) => metadata.is_dir(),
    Err(_) => false,
  }
}

/// Asynchronously gets the size of the file at the given path.
///
/// This is a soft check: it returns `0` (rather than an error) if the path
/// doesn't exist or its metadata cannot be read.
///
/// # Arguments
///
/// * `dest` - The path to the file.
///
/// # Returns
///
/// Returns the file size in bytes, or `0` if the file doesn't exist.
pub async fn get_file_size<S: AsRef<Path>>(dest: S) -> u64 {
  match fs::metadata(dest.as_ref()).await {
    Ok(metadata) => metadata.len(),
    Err(_) => 0,
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::fs::File;
  use std::io::Write;
  use tempfile::tempdir;

  #[tokio::test]
  async fn test_create_dir_all() -> AnyResult<()> {
    let dir = tempdir()?;
    let path = dir.path().join("test_dir");
    create_dir_all(&path).await?;
    assert!(path.exists());
    Ok(())
  }

  #[test]
  fn test_get_file_name_from_url_basic() -> AnyResult<()> {
    let url = "https://nodejs.org/dist/v22.11.0/node-v22.11.0.pkg";
    let file_name = get_file_name_from_url(url)?;
    assert_eq!("node-v22.11.0.pkg", file_name);
    Ok(())
  }

  #[test]
  fn test_get_file_name_from_url_with_query() -> AnyResult<()> {
    let url = "https://example.com/file.zip?v=1.0";
    let file_name = get_file_name_from_url(url)?;
    assert_eq!("file.zip", file_name);
    Ok(())
  }

  #[test]
  fn test_get_file_name_from_url_no_filename() -> AnyResult<()> {
    let url = "https://example.com/";
    let file_name = get_file_name_from_url(url)?;
    assert_eq!("", file_name);
    Ok(())
  }

  #[test]
  fn test_get_file_name_from_url_nested_path() -> AnyResult<()> {
    let url = "https://example.com/path/to/deep/file.txt";
    let file_name = get_file_name_from_url(url)?;
    assert_eq!("file.txt", file_name);
    Ok(())
  }

  #[test]
  fn test_gen_file_name() {
    let original = "test.txt";
    let generated = gen_file_name(original);
    // Should contain original filename
    assert!(generated.ends_with("test.txt"));
    // Should have a timestamp prefix
    let parts: Vec<&str> = generated.split('_').collect();
    assert!(parts.len() >= 2);
    // Timestamp must be filename-safe: no colons (Windows-illegal).
    assert!(
      !generated.starts_with(':'),
      "generated name should not start with a colon"
    );
  }

  #[tokio::test]
  async fn test_is_file_true() -> AnyResult<()> {
    let dir = tempdir()?;
    let file_path = dir.path().join("test.txt");
    let mut file = File::create(&file_path)?;
    file.write_all(b"test content")?;
    assert!(is_file(&file_path).await);
    Ok(())
  }

  #[tokio::test]
  async fn test_is_file_false_for_dir() -> AnyResult<()> {
    let dir = tempdir()?;
    assert!(!is_file(dir.path()).await);
    Ok(())
  }

  #[tokio::test]
  async fn test_is_file_false_nonexistent() -> AnyResult<()> {
    let path = Path::new("/nonexistent/path/file.txt");
    assert!(!is_file(path).await);
    Ok(())
  }

  #[tokio::test]
  async fn test_is_dir_true() -> AnyResult<()> {
    let dir = tempdir()?;
    assert!(is_dir(dir.path()).await);
    Ok(())
  }

  #[tokio::test]
  async fn test_is_dir_false_for_file() -> AnyResult<()> {
    let dir = tempdir()?;
    let file_path = dir.path().join("test.txt");
    File::create(&file_path)?;
    assert!(!is_dir(&file_path).await);
    Ok(())
  }

  #[tokio::test]
  async fn test_is_dir_false_nonexistent() -> AnyResult<()> {
    let path = Path::new("/nonexistent/path");
    assert!(!is_dir(path).await);
    Ok(())
  }

  #[tokio::test]
  async fn test_get_file_size_with_content() -> AnyResult<()> {
    let dir = tempdir()?;
    let file_path = dir.path().join("test.txt");
    let content = b"hello world";
    let mut file = File::create(&file_path)?;
    file.write_all(content)?;
    assert_eq!(content.len() as u64, get_file_size(&file_path).await);
    Ok(())
  }

  #[tokio::test]
  async fn test_get_file_size_empty_file() -> AnyResult<()> {
    let dir = tempdir()?;
    let file_path = dir.path().join("empty.txt");
    File::create(&file_path)?;
    assert_eq!(0, get_file_size(&file_path).await);
    Ok(())
  }

  #[tokio::test]
  async fn test_get_file_size_nonexistent() -> AnyResult<()> {
    let path = Path::new("/nonexistent/file.txt");
    assert_eq!(0, get_file_size(path).await);
    Ok(())
  }
}
