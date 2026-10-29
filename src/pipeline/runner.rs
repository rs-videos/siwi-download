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
//! println!("{} bytes", report.bytes_written);
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
  pub bytes_written: u64,
  /// Byte position the transfer resumed from.
  pub resumed_from: u64,
  /// Total payload size when the source reported one.
  pub total: Option<u64>,
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
      sink: &mut self.sink,
      filters: &mut self.filters,
      buf_in: Vec::new(),
      buf_out: Vec::new(),
    };
    let outcome = engine::pump(&mut self.source, &mut chain, spec).await?;
    chain.flush().await?;
    self.sink.finalize().await?;

    let report = PipelineReport {
      location: metadata.location,
      status: metadata.status,
      bytes_written: outcome.written,
      resumed_from: self.options.resume_from,
      total,
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
        .set_file_size(report.bytes_written)
        .set_range_from(report.resumed_from)
        .set_download_status(DownloadStatus::Complete);
      crate::download::dispatch_hooks(
        &self.options.hooks,
        &DownloadEvent::Complete { report: &legacy },
      )?;
    }

    Ok(report)
  }
}

/// A [`ChunkWriter`] that pipes chunks through the filter chain before
/// writing to the sink.
///
/// Two buffers ping-pong between filters; `flush` cascades each filter's
/// tail through the remaining filters and finally into the sink. With no
/// filters registered, chunks go straight to the sink.
struct FilterChain<'a, K: crate::download::sink::StreamSink> {
  sink: &'a mut K,
  filters: &'a mut [Box<dyn Filter>],
  /// Filter pipeline input buffer (swapped with `buf_out` after each stage).
  buf_in: Vec<u8>,
  /// Filter pipeline output buffer.
  buf_out: Vec<u8>,
}

impl<K: crate::download::sink::StreamSink> ChunkWriter for FilterChain<'_, K> {
  async fn write_chunk(&mut self, chunk: &[u8]) -> AnyResult<()> {
    if self.filters.is_empty() {
      return self.sink.write_chunk(chunk).await;
    }
    self.buf_in.clear();
    self.buf_in.extend_from_slice(chunk);
    for filter in self.filters.iter_mut() {
      self.buf_out.clear();
      filter.transform(&self.buf_in, &mut self.buf_out).await?;
      std::mem::swap(&mut self.buf_in, &mut self.buf_out);
    }
    if !self.buf_in.is_empty() {
      self.sink.write_chunk(&self.buf_in).await?;
    }
    Ok(())
  }
}

impl<K: crate::download::sink::StreamSink> FilterChain<'_, K> {
  /// Flushes every filter, cascading each tail through downstream stages.
  async fn flush(&mut self) -> AnyResult<()> {
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
        self.sink.write_chunk(&self.buf_in).await?;
      }
    }
    Ok(())
  }
}
