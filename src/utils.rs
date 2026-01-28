use crate::error::AnyResult;
use chrono::{DateTime, Utc};
use reqwest::Url;
use std::{borrow::Cow, path::Path};
use tokio::fs;

pub(crate) fn get_file_name_from_url<'a, S: AsRef<str>>(url: S) -> AnyResult<Cow<'a, str>> {
  let parse = Url::parse(url.as_ref())?;
  let file_name = parse
    .path_segments()
    .and_then(std::iter::Iterator::last)
    .unwrap_or("");
  Ok(Cow::Owned(file_name.to_owned()))
}

pub fn date() -> DateTime<Utc> {
  Utc::now()
}

pub fn gen_file_name<'a, S: Into<Cow<'a, str>>>(file_name: S) -> AnyResult<Cow<'a, str>> {
  let now = date().to_string();
  let new_file_name = format!("{}_{}", now, file_name.into());
  Ok(Cow::Owned(new_file_name))
}

pub async fn create_dir_all<S: AsRef<Path>>(src: S) -> AnyResult<()> {
  fs::create_dir_all(src.as_ref()).await?;
  Ok(())
}

pub fn is_file<S: AsRef<Path>>(dest: S) -> AnyResult<bool> {
  let mut result: bool = false;
  let maybe_file = Path::new(dest.as_ref());
  if let Ok(metadata) = maybe_file.metadata() {
    result = metadata.is_file();
  }
  Ok(result)
}

pub fn is_dir<S: AsRef<Path>>(dest: S) -> AnyResult<bool> {
  let mut result: bool = false;
  let maybe_file = Path::new(dest.as_ref());
  if let Ok(metadata) = maybe_file.metadata() {
    result = metadata.is_dir();
  }
  Ok(result)
}

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
