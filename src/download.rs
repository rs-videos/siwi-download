use crate::error::AnyResult;
use chrono::{DateTime, Utc};
use reqwest::{header::HeaderMap, Url};
use std::path::Path;
#[derive(Debug)]
pub struct DownloadOptions {
  pub file_name: Option<String>,
  pub proxy: Option<String>,
  pub resume: Option<bool>,
  pub headers: Option<HeaderMap>,
}
impl Default for DownloadOptions {
  fn default() -> Self {
    Self {
      file_name: None,
      proxy: None,
      resume: None,
      headers: None,
    }
  }
}

impl DownloadOptions {
  pub fn new() -> Self {
    Self {
      file_name: None,
      proxy: None,
      resume: None,
      headers: None,
    }
  }
  pub fn set_proxy<S: Into<String>>(&mut self, proxy: S) {
    self.proxy = Some(proxy.into());
  }
  pub fn set_headers(&mut self, headers: HeaderMap) {
    self.headers = Some(headers);
  }
  pub fn set_resume(&mut self, resume: bool) {
    self.resume = Some(resume);
  }
  pub fn set_file_name<S: Into<String>>(&mut self, file_name: S) {
    self.file_name = Some(file_name.into());
  }
  pub fn get_file_name(&self) -> Option<String> {
    self.file_name.clone()
  }
}
#[derive(Debug)]
pub struct DownloadReport {
  pub url: String,
  pub file_name: String,
  pub storage_path: String,
  pub file_path: String,
  pub file_size: Option<u64>,
  pub range_from: Option<u64>,
  pub download_start_at: Option<DateTime<Utc>>,
  pub download_end_at: Option<DateTime<Utc>>,
}

impl DownloadReport {
  pub fn new<S: Into<String>>(url: S, file_name: S, storage_path: S, file_path: S) -> Self {
    Self {
      url: url.into(),
      file_name: file_name.into(),
      storage_path: storage_path.into(),
      file_path: file_path.into(),
      file_size: None,
      range_from: None,
      download_start_at: None,
      download_end_at: None,
    }
  }

  pub fn set_file_size(&mut self, file_size: u64) {
    self.file_size = Some(file_size);
  }
  pub fn set_range_from(&mut self, range_from: u64) {
    self.range_from = Some(range_from);
  }
  pub fn set_download_start_at(&mut self) {
    self.download_start_at = Some(date());
  }

  pub fn set_download_end_at(&mut self) {
    self.download_end_at = Some(date());
  }
}

pub enum DownloadStatus {
  Pending,
  Complete,
  Error,
}
pub struct Download {
  pub url: String,
  pub storage_path: String,
}

impl Download {
  pub fn new<S: Into<String>>(url: S, storage_path: S) -> Self {
    Self {
      url: url.into(),
      storage_path: storage_path.into(),
    }
  }
  pub async fn download(&self, options: DownloadOptions) -> AnyResult<DownloadReport> {
    let url = self.url.clone();
    let storage_path = self.storage_path.clone();
    let mut file_name = get_file_name_from_url(url.as_str())?;
    if options.file_name.is_some() {
      file_name = options.file_name.unwrap();
    }
    let file_path = format!("{}/{}", storage_path.as_str(), file_name.as_str());
    let file_size = get_file_size(file_path.as_str())?;
    let mut report = DownloadReport::new(
      url,
      file_name.clone(),
      storage_path.clone(),
      file_path.clone(),
    );
    report.set_file_size(file_size);
    report.set_range_from(file_size);
    report.set_download_start_at();
    // 处理 自定义 headers

    // 处理是否覆盖原来的文件 断点下载

    // 处理进度条 一般生产上可以关闭

    // 真正下载
    report.set_download_end_at();
    //
    Ok(report)
  }
}

pub async fn download(options: DownloadOptions) -> AnyResult<()> {
  println!("options {:?}", options);
  Ok(())
}

fn get_file_name_from_url<S: Into<String>>(url: S) -> AnyResult<String> {
  let parsed = Url::parse(url.into().as_str())?;
  let file_name = parsed
    .path_segments()
    .and_then(std::iter::Iterator::last)
    .unwrap_or("");
  Ok(file_name.to_string())
}

fn get_file_size(file_path: &str) -> AnyResult<u64> {
  let mut file_size: u64 = 0;
  let path = Path::new(file_path);

  if let Ok(mate) = path.metadata() {
    file_size = mate.len();
  }
  Ok(file_size)
}

fn date() -> DateTime<Utc> {
  Utc::now()
}
