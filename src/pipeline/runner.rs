//! Pipeline runner: drives a [`Source`] through the shared transfer engine
//! into a [`Sink`](super::Sink), emitting the same lifecycle events as
//! `Download::download`.
//!
//! # Example
//!
//! ```rust,no_run
//! use siwi_download::download::sink::MemorySink;
//! use siwi_download::pipeline::{FileSource, Pipeline, PipelineOptions};
//!
//! # async fn example() -> siwi_download::error::AnyResult<()> {
//! let report = Pipeline::new(FileSource::new("/tmp/input.bin"), MemorySink::default())
//!     .options(PipelineOptions::default())
//!     .run()
//!     .await?;
//! println!("{} bytes", report.bytes_transferred);
//! # Ok(())
//! # }
//! ```

use super::filter::Filter;
use super::source::{Source, SourceMetadata};
use crate::download::engine::{self, ChunkWriter, PumpSpec};
use crate::download::events::{DownloadEvent, DownloadHook};
use crate::download::{DownloadOptions, DownloadReport, DownloadStatus};
use crate::error::AnyResult;
use chrono::{DateTime, Utc};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// Cross-cutting pipeline settings, mirroring the relevant subset of
/// [`DownloadOptions`].
#[derive(Default)]
pub struct PipelineOptions {
  /// Lifecycle hooks, invoked in registration order.
  pub hooks: Vec<Arc<dyn DownloadHook>>,
  /// Cooperative cancellation flag.
  pub cancel: Option<Arc<AtomicBool>>,
  /// Average-speed cap in bytes per second.
  pub max_speed: Option<u64>,
  /// Resume offset: ask the source to start at this byte position.
  pub resume_from: u64,
}

impl PipelineOptions {
  /// Carries hooks/cancel/speed over from [`DownloadOptions`].
  #[must_use]
  pub fn from_download_options(o: &DownloadOptions) -> Self {
    Self {
      hooks: o.hooks.clone(),
      cancel: o.cancel.clone(),
      max_speed: o.max_speed,
      resume_from: 0,
    }
  }
}

/// What a pipeline run produced.
#[derive(Debug, Clone)]
pub struct PipelineReport {
  /// Payload location (URL or path).
  pub location: String,
  /// Probe status (`0` for non-HTTP sources).
  pub status: u16,
  /// Bytes transferred by this run.
  pub bytes_transferred: u64,
  /// Bytes skipped because the transfer resumed mid-stream.
  pub bytes_skipped: u64,
  /// Full payload size when the source reported one.
  pub content_length: Option<u64>,
  /// Source entity tag, when available.
  pub etag: Option<String>,
  /// Source last-modified timestamp, when available.
  pub last_modified: Option<DateTime<Utc>>,
  /// True when cancellation stopped the transfer early.
  pub cancelled: bool,
}

/// A Source → Sink transfer.
pub struct Pipeline<S, K>
where
  S: Source,
  K: crate::download::sink::StreamSink,
{
  source: S,
  sink: K,
  options: PipelineOptions,
  filters: Vec<Box<dyn Filter>>,
}

impl<S, K> Pipeline<S, K>
where
  S: Source,
  K: crate::download::sink::StreamSink,
{
  /// Composes `source` with `sink`.
  pub fn new(source: S, sink: K) -> Self {
    Self {
      source,
      sink,
      options: PipelineOptions::default(),
      filters: Vec::new(),
    }
  }

  /// Appends a filter stage. Filters run in registration order; a filter
  /// error aborts the pipeline before the sink is finalized.
  #[must_use]
  pub fn filter(mut self, filter: Box<dyn Filter>) -> Self {
    self.filters.push(filter);
    self
  }

  /// Replaces the pipeline options.
  #[must_use]
  pub fn options(mut self, options: PipelineOptions) -> Self {
    self.options = options;
    self
  }

  /// Runs the transfer: probe, events, pump, events, report.
  ///
  /// # Errors
  ///
  /// Returns an error when the probe or the transfer fails (including a
  /// sink or hook error).
  pub async fn run(mut self) -> AnyResult<PipelineReport> {
    let metadata: SourceMetadata = self.source.metadata().await?;

    crate::download::dispatch_hooks(
      &self.options.hooks,
      &DownloadEvent::BeforeRequest {
        url: &metadata.location,
      },
    )?;
    crate::download::dispatch_hooks(
      &self.options.hooks,
      &DownloadEvent::HeadersReceived {
        url: &metadata.location,
        status: metadata.status,
        content_length: metadata.content_length,
      },
    )?;

    self.source.open(self.options.resume_from).await?;

    // `content_length` is the full payload size (the probe runs without a
    // Range header), so it is the total directly.
    let total = metadata.content_length;
    let spec = PumpSpec {
      hooks: &self.options.hooks,
      cancel: self.options.cancel.as_ref(),
      max_speed: self.options.max_speed,
      total,
      start_offset: self.options.resume_from,
      progress_bar: None,
    };
    let mut chain = FilterChain {
      core: FilterCore::new(std::mem::take(&mut self.filters)),
      sink: &mut self.sink,
    };
    let outcome = engine::pump(&mut self.source, &mut chain, spec).await?;
    chain.flush().await?;
    self.sink.finalize().await?;

    let report = PipelineReport {
      location: metadata.location,
      status: metadata.status,
      bytes_transferred: outcome.written,
      bytes_skipped: self.options.resume_from,
      content_length: total,
      etag: metadata.etag,
      last_modified: metadata.last_modified,
      cancelled: outcome.cancelled,
    };

    // Mirror the single-download event contract: Complete on success,
    // Error on cancellation.
    if report.cancelled {
      crate::download::dispatch_hooks(
        &self.options.hooks,
        &DownloadEvent::Error { msg: "cancelled" },
      )?;
    } else {
      let mut legacy = DownloadReport::new(
        report.location.clone(),
        String::new(),
        String::new(),
        String::new(),
        String::new(),
      );
      legacy
        .set_file_size(report.bytes_transferred)
        .set_range_from(report.bytes_skipped)
        .set_download_status(DownloadStatus::Complete);
      crate::download::dispatch_hooks(
        &self.options.hooks,
        &DownloadEvent::Complete { report: &legacy },
      )?;
    }

    Ok(report)
  }
}

/// Owned filter pipeline: buffers plus the filter stages.
///
/// Shared by [`FilterChain`] (borrowed sink, used by the pipeline runner)
/// and [`FilteredSink`] (owned sink, lets `Download::stream` users attach
/// filters without the pipeline types leaking into the core API).
struct FilterCore {
  filters: Vec<Box<dyn Filter>>,
  /// Pipeline input buffer (swapped with `buf_out` after each stage).
  buf_in: Vec<u8>,
  /// Pipeline output buffer.
  buf_out: Vec<u8>,
}

impl FilterCore {
  fn new(filters: Vec<Box<dyn Filter>>) -> Self {
    Self {
      filters,
      buf_in: Vec::new(),
      buf_out: Vec::new(),
    }
  }

  /// Runs `chunk` through every filter and writes the result to `sink`.
  async fn process<W: ChunkWriter>(&mut self, chunk: &[u8], sink: &mut W) -> AnyResult<()> {
    if self.filters.is_empty() {
      return sink.write_chunk(chunk).await;
    }
    self.buf_in.clear();
    self.buf_in.extend_from_slice(chunk);
    for filter in &mut self.filters {
      self.buf_out.clear();
      filter.transform(&self.buf_in, &mut self.buf_out).await?;
      std::mem::swap(&mut self.buf_in, &mut self.buf_out);
    }
    if !self.buf_in.is_empty() {
      sink.write_chunk(&self.buf_in).await?;
    }
    Ok(())
  }

  /// Flushes every filter, cascading each tail through downstream stages.
  async fn flush<W: ChunkWriter>(&mut self, sink: &mut W) -> AnyResult<()> {
    let n = self.filters.len();
    for i in 0..n {
      self.buf_out.clear();
      {
        let (_, tail) = self.filters.split_at_mut(i);
        tail[0].flush(&mut self.buf_out).await?;
      }
      if self.buf_out.is_empty() {
        continue;
      }
      std::mem::swap(&mut self.buf_in, &mut self.buf_out);
      for j in (i + 1)..n {
        self.buf_out.clear();
        {
          let (_, tail) = self.filters.split_at_mut(j);
          tail[0].transform(&self.buf_in, &mut self.buf_out).await?;
        }
        std::mem::swap(&mut self.buf_in, &mut self.buf_out);
      }
      if !self.buf_in.is_empty() {
        sink.write_chunk(&self.buf_in).await?;
      }
    }
    Ok(())
  }
}

/// Adapts the filter core to the engine's writer contract for the runner.
struct FilterChain<'a, K: crate::download::sink::StreamSink> {
  core: FilterCore,
  sink: &'a mut K,
}

impl<K: crate::download::sink::StreamSink> ChunkWriter for FilterChain<'_, K> {
  async fn write_chunk(&mut self, chunk: &[u8]) -> AnyResult<()> {
    self.core.process(chunk, self.sink).await
  }
}

impl<K: crate::download::sink::StreamSink> FilterChain<'_, K> {
  async fn flush(&mut self) -> AnyResult<()> {
    self.core.flush(self.sink).await
  }
}

/// A [`Sink`](crate::download::sink::StreamSink) wrapper that runs incoming
/// bytes through a filter chain before forwarding them.
///
/// This lets `Download::stream` (and any other sink consumer) use filters
/// without touching the core API:
///
/// ```rust,no_run
/// use siwi_download::download::sink::MemorySink;
/// use siwi_download::download::Algorithm;
/// use siwi_download::pipeline::{ChecksumFilter, FilteredSink};
///
/// # async fn example() -> siwi_download::error::AnyResult<()> {
/// let mut sink = FilteredSink::new(MemorySink::default())
///     .push(Box::new(ChecksumFilter::new(Algorithm::Sha256, "00".repeat(32))) as Box<dyn siwi_download::pipeline::Filter>);
/// // download.stream(url, options, &mut sink).await?;
/// # Ok(())
/// # }
/// ```
pub struct FilteredSink<S: crate::download::sink::StreamSink> {
  core: FilterCore,
  inner: S,
}

impl<S: crate::download::sink::StreamSink> FilteredSink<S> {
  /// Wraps `inner` with an empty filter chain.
  pub fn new(inner: S) -> Self {
    Self {
      core: FilterCore::new(Vec::new()),
      inner,
    }
  }

  /// Appends a filter stage (runs in registration order).
  #[must_use]
  pub fn push(mut self, filter: Box<dyn Filter>) -> Self {
    self.core.filters.push(filter);
    self
  }

  /// Access to the wrapped sink.
  pub fn inner(&self) -> &S {
    &self.inner
  }
}

impl<S: crate::download::sink::StreamSink> crate::download::sink::StreamSink for FilteredSink<S> {
  async fn write_chunk(&mut self, chunk: &[u8]) -> AnyResult<()> {
    self.core.process(chunk, &mut self.inner).await
  }

  async fn finalize(&mut self) -> AnyResult<()> {
    self.core.flush(&mut self.inner).await?;
    self.inner.finalize().await
  }
}
