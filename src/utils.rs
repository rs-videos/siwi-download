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

pub fn is_file<'a, S: AsRef<Path>>(dest: S) -> AnyResult<bool> {
  let mut result: bool = false;
  let maybe_file = Path::new(dest.as_ref());
  if let Ok(metadata) = maybe_file.metadata() {
    result = metadata.is_file();
  }
  Ok(result)
}

pub fn is_dir<'a, S: AsRef<Path>>(dest: S) -> AnyResult<bool> {
  let mut result: bool = false;
  let maybe_file = Path::new(dest.as_ref());
  if let Ok(metadata) = maybe_file.metadata() {
    result = metadata.is_dir();
  }
  Ok(result)
}

pub fn get_file_size<'a, S: AsRef<Path>>(dest: S) -> AnyResult<u64> {
  let mut result: u64 = 0;
  let maybe_file = Path::new(dest.as_ref());
  if let Ok(metadata) = maybe_file.metadata() {
    result = metadata.len();
  }
  Ok(result)
}
