//! Utility functions for URL parsing, file operations, and timestamp handling.
//!
//! This module provides helper functions used throughout siwi-download
//! for extracting filenames from URLs, checking file/directory status,
//! and managing file system operations.

use crate::error::AnyResult;
use chrono::{DateTime, Utc};
use reqwest::Url;
use std::{borrow::Cow, path::Path};
use tokio::fs;

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
/// Returns a [`Cow`] containing the filename, or an empty string if
/// the URL has no path segments.
pub(crate) fn get_file_name_from_url<'a, S: AsRef<str>>(url: S) -> AnyResult<Cow<'a, str>> {
  let parse = Url::parse(url.as_ref())?;
  let file_name = parse
    .path_segments()
    .and_then(std::iter::Iterator::last)
    .unwrap_or("");
  Ok(Cow::Owned(file_name.to_owned()))
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
/// # Arguments
///
/// * `file_name` - The original filename.
///
/// # Returns
///
/// Returns a [`Cow`] containing the new filename in the format
/// `{timestamp}_{original_filename}`.
///
/// # Example
///
/// ```rust
/// use siwi_download::utils::gen_file_name;
///
/// let new_name = gen_file_name("document.pdf").unwrap();
/// assert!(new_name.to_string().ends_with("document.pdf"));
/// ```
pub fn gen_file_name<'a, S: Into<Cow<'a, str>>>(file_name: S) -> AnyResult<Cow<'a, str>> {
  let now = date().to_string();
  let new_file_name = format!("{}_{}", now, file_name.into());
  Ok(Cow::Owned(new_file_name))
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
pub async fn create_dir_all<S: AsRef<Path>>(src: S) -> AnyResult<()> {
  fs::create_dir_all(src.as_ref()).await?;
  Ok(())
}

/// Checks if the given path points to a file.
///
/// This function safely checks whether the path exists and is a regular file.
/// Unlike [`std::fs::metadata`], this function returns `false` instead of
/// an error if the path doesn't exist or cannot be accessed.
///
/// # Arguments
///
/// * `dest` - The path to check.
///
/// # Returns
///
/// Returns `true` if the path exists and is a file, `false` otherwise.
pub fn is_file<S: AsRef<Path>>(dest: S) -> AnyResult<bool> {
  let mut result: bool = false;
  let maybe_file = Path::new(dest.as_ref());
  if let Ok(metadata) = maybe_file.metadata() {
    result = metadata.is_file();
  }
  Ok(result)
}

/// Checks if the given path points to a directory.
///
/// This function safely checks whether the path exists and is a directory.
/// Returns `false` instead of an error if the path doesn't exist.
///
/// # Arguments
///
/// * `dest` - The path to check.
///
/// # Returns
///
/// Returns `true` if the path exists and is a directory, `false` otherwise.
pub fn is_dir<S: AsRef<Path>>(dest: S) -> AnyResult<bool> {
  let mut result: bool = false;
  let maybe_file = Path::new(dest.as_ref());
  if let Ok(metadata) = maybe_file.metadata() {
    result = metadata.is_dir();
  }
  Ok(result)
}

/// Gets the size of the file at the given path.
///
/// This function returns the file size in bytes. If the path doesn't
/// exist or is not a file, it returns 0.
///
/// # Arguments
///
/// * `dest` - The path to the file.
///
/// # Returns
///
/// Returns the file size in bytes, or 0 if the file doesn't exist.
pub fn get_file_size<S: AsRef<Path>>(dest: S) -> AnyResult<u64> {
  let mut result: u64 = 0;
  let maybe_file = Path::new(dest.as_ref());
  if let Ok(metadata) = maybe_file.metadata() {
    result = metadata.len();
  }
  Ok(result)
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
  fn test_gen_file_name() -> AnyResult<()> {
    let original = "test.txt";
    let generated = gen_file_name(original)?;
    // Should contain original filename
    assert!(generated.to_string().ends_with("test.txt"));
    // Should have a timestamp prefix
    let parts: Vec<&str> = generated.split('_').collect();
    assert!(parts.len() >= 2);
    Ok(())
  }

  #[test]
  fn test_is_file_true() -> AnyResult<()> {
    let dir = tempdir()?;
    let file_path = dir.path().join("test.txt");
    let mut file = File::create(&file_path)?;
    file.write_all(b"test content")?;
    assert!(is_file(&file_path)?);
    Ok(())
  }

  #[test]
  fn test_is_file_false_for_dir() -> AnyResult<()> {
    let dir = tempdir()?;
    assert!(!is_file(dir.path())?);
    Ok(())
  }

  #[test]
  fn test_is_file_false_nonexistent() -> AnyResult<()> {
    let path = Path::new("/nonexistent/path/file.txt");
    assert!(!is_file(path)?);
    Ok(())
  }

  #[test]
  fn test_is_dir_true() -> AnyResult<()> {
    let dir = tempdir()?;
    assert!(is_dir(dir.path())?);
    Ok(())
  }

  #[test]
  fn test_is_dir_false_for_file() -> AnyResult<()> {
    let dir = tempdir()?;
    let file_path = dir.path().join("test.txt");
    File::create(&file_path)?;
    assert!(!is_dir(&file_path)?);
    Ok(())
  }

  #[test]
  fn test_is_dir_false_nonexistent() -> AnyResult<()> {
    let path = Path::new("/nonexistent/path");
    assert!(!is_dir(path)?);
    Ok(())
  }

  #[test]
  fn test_get_file_size_with_content() -> AnyResult<()> {
    let dir = tempdir()?;
    let file_path = dir.path().join("test.txt");
    let content = b"hello world";
    let mut file = File::create(&file_path)?;
    file.write_all(content)?;
    assert_eq!(content.len() as u64, get_file_size(&file_path)?);
    Ok(())
  }

  #[test]
  fn test_get_file_size_empty_file() -> AnyResult<()> {
    let dir = tempdir()?;
    let file_path = dir.path().join("empty.txt");
    File::create(&file_path)?;
    assert_eq!(0, get_file_size(&file_path)?);
    Ok(())
  }

  #[test]
  fn test_get_file_size_nonexistent() -> AnyResult<()> {
    let path = Path::new("/nonexistent/file.txt");
    assert_eq!(0, get_file_size(path)?);
    Ok(())
  }
}
