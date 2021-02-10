use crate::error::AnyResult;
use chrono::{DateTime, Utc};
use indicatif::{ProgressBar, ProgressStyle};
use reqwest::{
  header::{HeaderMap, HeaderValue, RANGE},
  Url,
};
use std::path::Path;
use tokio::{fs, io::AsyncWriteExt};
#[derive(Debug)]
pub struct DownloadOptions {
  pub maybe_file_name: Option<String>,
  pub maybe_proxy: Option<String>,
  pub maybe_headers: Option<HeaderMap>,
  pub show_progress: bool,
}
impl Default for DownloadOptions {
  fn default() -> Self {
    Self {
      maybe_file_name: None,
      maybe_proxy: None,
      maybe_headers: None,
      show_progress: false,
    }
  }
}

impl DownloadOptions {
  pub fn new() -> Self {
    Self {
      maybe_file_name: None,
      maybe_proxy: None,
      maybe_headers: None,
      show_progress: false,
    }
  }
  pub fn set_proxy<S: Into<String>>(&mut self, proxy: S) -> &mut DownloadOptions {
    self.maybe_proxy = Some(proxy.into());
    self
  }
  pub fn set_headers(&mut self, headers: HeaderMap) -> &mut DownloadOptions {
    self.maybe_headers = Some(headers);
    self
  }
  pub fn set_show_progress(&mut self, show_progress: bool) -> &mut DownloadOptions {
    self.show_progress = show_progress;
    self
  }
  pub fn set_file_name<S: Into<String>>(&mut self, file_name: S) -> &mut DownloadOptions {
    self.maybe_file_name = Some(file_name.into());
    self
  }
  pub fn get_file_name(&self) -> &Option<String> {
    &self.maybe_file_name
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
  pub download_status: Option<DownloadStatus>,
  pub headers: Option<HeaderMap>,
  pub head_status: Option<u16>,
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
      download_status: Some(DownloadStatus::Create),
      headers: None,
      head_status: None,
    }
  }

  pub fn set_file_size(&mut self, file_size: u64) -> &mut Self {
    self.file_size = Some(file_size);
    self
  }
  pub fn set_range_from(&mut self, range_from: u64) -> &mut Self {
    self.range_from = Some(range_from);
    self
  }
  pub fn set_download_start_at(&mut self) -> &mut Self {
    self.download_start_at = Some(date());
    self
  }
  pub fn set_download_end_at(&mut self) -> &mut Self {
    self.download_end_at = Some(date());
    self
  }
  pub fn set_download_status(&mut self, status: DownloadStatus) -> &mut Self {
    self.download_status = Some(status);
    self
  }
  pub fn set_headers(&mut self, headers: HeaderMap) -> &mut Self {
    self.headers = Some(headers);
    self
  }
  pub fn set_head_status(&mut self, head_status: u16) -> &mut Self {
    self.head_status = Some(head_status);
    self
  }
}
#[derive(Debug)]
pub enum DownloadStatus {
  Create,
  Append,
  Complete,
  Exists,
  Error
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

    let file_name = match options.maybe_file_name {
      Some(file_name) => file_name,
      None => get_file_name_from_url(url.as_str())?,
    };

    let file_path = format!("{}/{}", storage_path.as_str(), file_name.as_str());
    let file_size = get_file_size(file_path.as_str())?;

    let mut report = DownloadReport::new(
      url.as_str(),
      file_name.as_str(),
      storage_path.as_str(),
      file_path.as_str(),
    );

    report
      .set_file_size(file_size)
      .set_range_from(file_size)
      .set_download_start_at();

    // 处理 自定义 headers
    let mut headers = match options.maybe_headers {
      Some(headers) => headers,
      None => HeaderMap::new(),
    };

    let range = format!("bytes={}-", file_size);
    headers.insert(RANGE, HeaderValue::from_str(range.as_str())?);
    report.set_headers(headers.clone());
    // client
    let client = match options.maybe_proxy {
      Some(proxy) => reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(proxy.as_str())?)
        .default_headers(headers)
        .build()?,
      None => reqwest::Client::builder()
        .no_proxy()
        .default_headers(headers)
        .build()?,
    };
    // head 获取文件大小
    let resp = client.head(url.as_str()).send().await?;
    let status = resp.status().as_u16();
    report.set_head_status(status);
    if status > 300 {
      report.set_download_status(DownloadStatus::Exists);
      report.set_download_end_at();
      return Ok(report);
    }

    let content_length = match resp.content_length() {
      Some(len) => len,
      None => 0,
    };

    let total = file_size + content_length;
    report.set_file_size(total);
    let pb = ProgressBar::new(total);
    if options.show_progress {
      pb.set_style(
        ProgressStyle::default_bar()
          .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta})")
          .progress_chars("#>-"),
      );
    }

    if file_size > 0 {
      report.set_download_status(DownloadStatus::Append);
      if options.show_progress {
        pb.set_position(file_size);
      }
    }

    let mut resp = client.get(url.as_str()).send().await?;
    let mut dest = fs::OpenOptions::new()
      .create(true)
      .append(true)
      .open(&file_path)
      .await?;

    while let Some(chunk) = resp.chunk().await? {
      dest.write_all(&chunk).await?;
      if options.show_progress {
        pb.inc(chunk.len() as u64);
      }
    }

    report.set_download_status(DownloadStatus::Complete);
    report.set_download_end_at();

    Ok(report)
  }
}

pub fn get_file_name_from_url<S: Into<String>>(url: S) -> AnyResult<String> {
  let parsed = Url::parse(url.into().as_str())?;
  let file_name = parsed
    .path_segments()
    .and_then(std::iter::Iterator::last)
    .unwrap_or("tmp_name_file");
  Ok(file_name.to_string())
}

pub fn get_file_size(file_path: &str) -> AnyResult<u64> {
  let mut file_size: u64 = 0;
  let path = Path::new(file_path);

  if let Ok(mate) = path.metadata() {
    file_size = mate.len();
  }
  Ok(file_size)
}

pub fn date() -> DateTime<Utc> {
  Utc::now()
}

pub fn gen_file_name<S: Into<String>>(file_name: S) -> AnyResult<String> {
  let mut new_file_name = file_name.into();
  let now = date().to_string();
  new_file_name = format!("{}_{}", now, new_file_name);
  Ok(new_file_name)
}

pub async fn create_dir_all(src: &str) -> AnyResult<()> {
  fs::create_dir_all(src).await?;
  Ok(())
}
