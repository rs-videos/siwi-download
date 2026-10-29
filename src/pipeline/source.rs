//! Sources: where pipeline bytes come from (HTTP, local files).

use crate::download::DownloadOptions;
use crate::download::client;
use crate::download::engine::ChunkSource;
use crate::download::events::{DownloadEvent, DownloadHook};
use crate::error::AnyResult;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use reqwest::header::{
  CONTENT_LENGTH, HeaderMap, HeaderValue, IF_MODIFIED_SINCE, IF_NONE_MATCH, RANGE,
};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::time::sleep;
use tracing::{info, warn};

/// HTTP status codes the probe treats as usable answers.
const HTTP_OK: u16 = 200;
/// Partial content — the server honored our resume range.
const HTTP_PARTIAL_CONTENT: u16 = 206;
/// Terminal "already complete" answer to a ranged probe.
const HTTP_RANGE_NOT_SATISFIABLE: u16 = 416;
/// Terminal "nothing newer" answer to a conditional probe.
const HTTP_NOT_MODIFIED: u16 = 304;
/// Probe retry budget, matching `Download::download`.
const MAX_PROBE_RETRIES: u32 = 5;
/// Delay between probe retries.
const PROBE_RETRY_DELAY: Duration = Duration::from_secs(3);
/// Read buffer for [`FileSource`].
const FILE_CHUNK: usize = 64 * 1024;

/// What a [`Source`] reports about the payload before transfer.
#[derive(Debug, Clone)]
pub struct SourceMetadata {
  /// Probe location (URL or path), used in lifecycle events and reports.
  pub location: String,
  /// HTTP status of the probe (`0` for non-HTTP sources).
  pub status: u16,
  /// Total payload size in bytes, when known.
  pub content_length: Option<u64>,
  /// Server-provided entity tag.
  pub etag: Option<String>,
  /// Server-provided last-modified timestamp.
  pub last_modified: Option<DateTime<Utc>>,
  /// `true` when the source can honor a non-zero `range_from`.
  pub accepts_range: bool,
}

/// A byte source for a [`Pipeline`](super::Pipeline).
///
/// The contract is pull-based: [`Source::open`] prepares a transfer starting
/// at `range_from`, then [`Source::next_chunk`] yields `Ok(None)` at end of
/// stream. Implementations are `Send` so the pipeline can run on any tokio
/// task.
pub trait Source: Send {
  /// Probes the source without transferring payload.
  fn metadata(&self) -> impl Future<Output = AnyResult<SourceMetadata>> + Send;

  /// Prepares the transfer. `range_from > 0` requests a mid-stream start
  /// when the source supports ranges.
  fn open(&mut self, range_from: u64) -> impl Future<Output = AnyResult<()>> + Send;

  /// Fetches the next chunk, or `None` at end of stream.
  fn next_chunk(&mut self) -> impl Future<Output = AnyResult<Option<Bytes>>> + Send;
}

/// HTTP(S) source.
///
/// Options are the HTTP-relevant subset of
/// [`DownloadOptions`]: proxy, extra
/// headers, and conditional-request settings.
#[derive(Clone, Default)]
pub struct HttpSourceOptions {
  /// Proxy URL.
  pub proxy: Option<String>,
  /// Extra request headers.
  pub headers: Option<HeaderMap>,
  /// `If-Modified-Since` precondition.
  pub if_modified_since: Option<DateTime<Utc>>,
  /// `If-None-Match` precondition.
  pub if_none_match: Option<String>,
  /// Hooks for probe retries (`Retry` events).
  pub hooks: Vec<Arc<dyn DownloadHook>>,
  /// Delay between probe retries; defaults to 3s. Shorten in tests.
  pub retry_delay: Option<Duration>,
  /// Probe retry budget; defaults to 5. Shorten in tests.
  pub max_retries: Option<u32>,
}

impl std::fmt::Debug for HttpSourceOptions {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("HttpSourceOptions")
      .field("proxy", &self.proxy)
      .field("headers", &self.headers)
      .field("if_modified_since", &self.if_modified_since)
      .field("if_none_match", &self.if_none_match)
      .field("hooks", &self.hooks.len())
      .field("retry_delay", &self.retry_delay)
      .field("max_retries", &self.max_retries)
      .finish()
  }
}

impl HttpSourceOptions {
  /// Extracts the HTTP-relevant options from a [`DownloadOptions`].
  #[must_use]
  pub fn from_download_options(o: &DownloadOptions) -> Self {
    Self {
      proxy: o.maybe_proxy.clone(),
      headers: o.maybe_headers.clone(),
      if_modified_since: o.maybe_if_modified_since,
      if_none_match: o.maybe_if_none_match.clone(),
      hooks: o.hooks.clone(),
      retry_delay: None,
      max_retries: None,
    }
  }
}

/// Downloads over HTTP(S).
pub struct HttpSource {
  url: String,
  options: HttpSourceOptions,
  client: reqwest::Client,
  range_from: u64,
  resp: Option<reqwest::Response>,
}

impl HttpSource {
  /// Creates a source for `url`.
  ///
  /// # Errors
  ///
  /// Returns an error when the HTTP client cannot be built (invalid proxy).
  pub fn new(url: impl Into<String>, options: HttpSourceOptions) -> AnyResult<Self> {
    // Per-request headers carry the caller's headers; the client stays
    // header-free so nothing is sent twice.
    let client = client::build_client(options.proxy.as_deref(), HeaderMap::new())?;
    Ok(Self {
      url: url.into(),
      options,
      client,
      range_from: 0,
      resp: None,
    })
  }

  /// The URL this source reads.
  #[must_use]
  pub fn url(&self) -> &str {
    &self.url
  }

  fn request_headers(&self, ranged: bool) -> AnyResult<HeaderMap> {
    let mut headers = self.options.headers.clone().unwrap_or_default();
    if ranged && self.range_from > 0 {
      let range = format!("bytes={}-", self.range_from);
      headers.insert(RANGE, HeaderValue::from_str(&range)?);
    }
    if let Some(since) = self.options.if_modified_since {
      headers.insert(
        IF_MODIFIED_SINCE,
        HeaderValue::from_str(&crate::download::format_http_date(since))?,
      );
    }
    if let Some(etag) = self.options.if_none_match.as_ref() {
      headers.insert(IF_NONE_MATCH, HeaderValue::from_str(etag)?);
    }
    Ok(headers)
  }
}

/// `true` when a probe status is a terminal, usable answer.
fn acceptable(status: u16) -> bool {
  matches!(
    status,
    HTTP_OK | HTTP_PARTIAL_CONTENT | HTTP_NOT_MODIFIED | HTTP_RANGE_NOT_SATISFIABLE
  )
}

impl Source for HttpSource {
  async fn metadata(&self) -> AnyResult<SourceMetadata> {
    let mut resp = self
      .client
      .head(&self.url)
      .headers(self.request_headers(true)?)
      .send()
      .await?;
    let mut status = resp.status().as_u16();
    let mut attempt: u32 = 0;
    let retry_delay = self.options.retry_delay.unwrap_or(PROBE_RETRY_DELAY);
    let max_retries = self.options.max_retries.unwrap_or(MAX_PROBE_RETRIES);
    while !acceptable(status) && attempt < max_retries {
      attempt += 1;
      warn!(
        attempt,
        status, "probe returned non-acceptable status, retrying"
      );
      crate::download::dispatch_hooks(
        &self.options.hooks,
        &DownloadEvent::Retry { attempt, status },
      )?;
      sleep(retry_delay).await;
      resp = self
        .client
        .head(&self.url)
        .headers(self.request_headers(true)?)
        .send()
        .await?;
      status = resp.status().as_u16();
    }

    let content_length = resp
      .headers()
      .get(CONTENT_LENGTH)
      .and_then(|hv| hv.to_str().ok())
      .and_then(|l| l.parse::<u64>().ok());
    let etag = resp
      .headers()
      .get(reqwest::header::ETAG)
      .and_then(|hv| hv.to_str().ok())
      .map(ToOwned::to_owned);
    let last_modified = resp
      .headers()
      .get(reqwest::header::LAST_MODIFIED)
      .and_then(|hv| hv.to_str().ok())
      .and_then(|raw| chrono::DateTime::parse_from_rfc2822(raw).ok())
      .map(|dt| dt.with_timezone(&Utc));

    Ok(SourceMetadata {
      location: self.url.clone(),
      status,
      content_length,
      etag,
      last_modified,
      accepts_range: status == HTTP_PARTIAL_CONTENT || status == HTTP_RANGE_NOT_SATISFIABLE,
    })
  }

  async fn open(&mut self, range_from: u64) -> AnyResult<()> {
    self.range_from = range_from;
    let resp = self
      .client
      .get(&self.url)
      .headers(self.request_headers(true)?)
      .send()
      .await?;
    let status = resp.status().as_u16();
    if status >= 300 && status != HTTP_NOT_MODIFIED && status != HTTP_RANGE_NOT_SATISFIABLE {
      return Err(anyhow::anyhow!("HTTP {status} for {}", self.url));
    }
    info!(url = %self.url, status, range_from, "pipeline source opened");
    self.resp = Some(resp);
    Ok(())
  }

  async fn next_chunk(&mut self) -> AnyResult<Option<Bytes>> {
    match self.resp.as_mut() {
      Some(resp) => Ok(resp.chunk().await?),
      None => Err(anyhow::anyhow!("source not opened; call open() first")),
    }
  }
}

/// Every pipeline [`Source`] is directly usable by the transfer engine.
impl<S: Source> ChunkSource for S {
  async fn next_chunk(&mut self) -> AnyResult<Option<Bytes>> {
    Source::next_chunk(self).await
  }
}

/// Reads bytes from a local file.
///
/// Useful as a test fixture source and for file → sink transfers (e.g.
/// future `file → S3` uploads).
pub struct FileSource {
  path: PathBuf,
  file: Option<tokio::fs::File>,
}

impl FileSource {
  /// Creates a source for `path`.
  #[must_use]
  pub fn new(path: impl AsRef<Path>) -> Self {
    Self {
      path: path.as_ref().to_path_buf(),
      file: None,
    }
  }

  /// The path this source reads.
  #[must_use]
  pub fn path(&self) -> &Path {
    &self.path
  }
}

impl Source for FileSource {
  async fn metadata(&self) -> AnyResult<SourceMetadata> {
    let meta = tokio::fs::metadata(&self.path).await?;
    Ok(SourceMetadata {
      location: self.path.display().to_string(),
      status: 0,
      content_length: Some(meta.len()),
      etag: None,
      last_modified: None,
      accepts_range: true,
    })
  }

  async fn open(&mut self, range_from: u64) -> AnyResult<()> {
    let mut file = tokio::fs::File::open(&self.path).await?;
    if range_from > 0 {
      file.seek(std::io::SeekFrom::Start(range_from)).await?;
    }
    self.file = Some(file);
    Ok(())
  }

  async fn next_chunk(&mut self) -> AnyResult<Option<Bytes>> {
    let file = self
      .file
      .as_mut()
      .ok_or_else(|| anyhow::anyhow!("source not opened; call open() first"))?;
    let mut buf = vec![0u8; FILE_CHUNK];
    let n = file.read(&mut buf).await?;
    if n == 0 {
      return Ok(None);
    }
    buf.truncate(n);
    Ok(Some(Bytes::from(buf)))
  }
}
