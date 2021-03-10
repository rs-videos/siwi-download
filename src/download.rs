use crate::{
  error::AnyResult,
  utils::{create_dir_all, date, get_file_name_from_url, get_file_size, is_dir},
};
use chrono::{DateTime, Utc};
use indicatif::{ProgressBar, ProgressStyle};
use log::info;
use reqwest::header::{HeaderMap, HeaderValue, RANGE};
use std::{borrow::Cow, path::Path};
use tokio::time::sleep;
use tokio::time::Duration;
use tokio::{fs, io::AsyncWriteExt};
#[derive(Debug)]
pub struct DownloadOptions<'a> {
  pub maybe_file_name: Option<Cow<'a, str>>,
  pub maybe_proxy: Option<Cow<'a, str>>,
  pub maybe_headers: Option<HeaderMap>,
  pub show_progress: bool,
}
impl<'a> Default for DownloadOptions<'a> {
  fn default() -> Self {
    Self {
      maybe_file_name: None,
      maybe_proxy: None,
      maybe_headers: None,
      show_progress: false,
    }
  }
}

impl<'a> DownloadOptions<'a> {
  pub fn new() -> Self {
    Self {
      maybe_file_name: None,
      maybe_proxy: None,
      maybe_headers: None,
      show_progress: false,
    }
  }
  pub fn set_proxy<S>(&mut self, proxy: S) -> &mut Self
  where
    S: Into<Cow<'a, str>>,
  {
    self.maybe_proxy = Some(proxy.into());
    self
  }
  pub fn set_headers(&mut self, headers: HeaderMap) -> &mut Self {
    self.maybe_headers = Some(headers);
    self
  }
  pub fn set_show_progress(&mut self, show_progress: bool) -> &mut Self {
    self.show_progress = show_progress;
    self
  }
  pub fn set_file_name<S: Into<Cow<'a, str>>>(&mut self, file_name: S) -> &mut Self {
    self.maybe_file_name = Some(file_name.into());
    self
  }
}
#[derive(Debug)]
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
  pub headers: Option<HeaderMap>,
  pub head_status: Option<u16>,
  pub resp_status: Option<u16>,
  pub time_used: Option<i64>,
  pub msg: Option<Cow<'a, str>>,
}

impl<'a> DownloadReport<'a> {
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
  pub fn set_resp_status(&mut self, resp_status: u16) -> &mut Self {
    self.resp_status = Some(resp_status);
    self
  }
  pub fn set_msg<S: Into<Cow<'a, str>>>(&mut self, msg: S) -> &mut Self {
    self.msg = Some(msg.into());
    self
  }
  pub fn gen_time_used(&mut self) -> &mut Self {
    let mut time_used: i64 = 0;
    if let Some(end) = self.download_end_at {
      if let Some(start) = self.download_start_at {
        time_used = end.timestamp() - start.timestamp();
      };
    }
    self.time_used = Some(time_used);
    self
  }

  pub async fn report(&self, url: &str, headers: HeaderMap) -> AnyResult<Cow<'a, str>> {
    let client = reqwest::Client::builder()
      .no_proxy()
      .default_headers(headers)
      .build()?;
    let res = client.post(url).send().await?.text().await?;

    Ok(Cow::Owned(res.to_string()))
  }
}

#[derive(Debug)]
pub enum DownloadStatus {
  Create,
  Append,
  Complete,
  Exists,
  Error,
}

pub struct Download<'a> {
  pub storage_path: Cow<'a, str>,
}

impl<'a> Download<'a> {
  pub fn new<S: Into<Cow<'a, str>>>(storage_path: S) -> Self {
    Self {
      storage_path: storage_path.into(),
    }
  }

  pub async fn auto_create_storage_path(&self) -> AnyResult<()> {
    if !is_dir(self.storage_path.as_ref())? {
      match create_dir_all(self.storage_path.as_ref()).await {
        Ok(()) => println!("create storage_path {}", &self.storage_path),
        Err(e) => eprint!("create storage_path err {:?}", e),
      }
    }
    Ok(())
  }

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
        .proxy(reqwest::Proxy::all(proxy.as_ref())?)
        .default_headers(headers)
        .build()?,
      None => reqwest::Client::builder()
        .no_proxy()
        .default_headers(headers)
        .build()?,
    };
    // head 获取文件大小
    let try_times_limit: u16 = 5;
    let mut this_time: u16 = 0;

    let mut resp = client.head(url.as_ref()).send().await?;
    let mut status = resp.status().as_u16();
    if status != 206 || status != 416 {
      info!("head status is {} start loop", status);
      while this_time < try_times_limit {
        resp = client.head(url.as_ref()).send().await?;
        status = resp.status().as_u16();
        info!("try {} time head status is {}", this_time, status);
        if status != 206 || status != 416 {
          break;
        }
        this_time += 1;
        sleep(Duration::from_secs(3)).await;
      }
    }

    report.set_head_status(status);

    if status > 300 {
      if status == 416 {
        report.set_download_status(DownloadStatus::Exists);
        report.set_download_end_at();
        return Ok(report);
      } else {
        report.set_download_status(DownloadStatus::Error);
        report.set_download_end_at();
        return Ok(report);
      }
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

    let mut resp = client.get(url.as_ref()).send().await?;
    let status = resp.status().as_u16();
    report.set_resp_status(status);

    if status > 300 {
      if status == 416 {
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

    report.set_download_status(DownloadStatus::Complete);
    report.set_download_end_at().gen_time_used();

    Ok(report)
  }
}
