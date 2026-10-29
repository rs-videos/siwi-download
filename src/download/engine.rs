//! Shared transfer engine.
//!
//! One byte-pump implementation backs `Download::download`,
//! `Download::stream`, and (3.0) the generic `Pipeline`, so lifecycle
//! events, progress rate-limiting, the average-speed cap, cooperative
//! cancellation, and progress-bar updates cannot drift between code paths.
//!
//! # Design
//!
//! The pump is pull-based over two small traits ([`ChunkSource`] /
//! [`ChunkWriter`]) rather than async closures: closures that lend `&mut`
//! state across `.await` hit higher-ranked-lifetime limits when the future
//! is spawned, while trait-based pull iteration stays `Send` and simple.

use super::events::{self, DownloadEvent, DownloadHook};
use super::options::DownloadOptions;
use crate::error::AnyResult;
use bytes::Bytes;
use indicatif::ProgressBar;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use tokio::time::{Duration, sleep};
use tracing::info;

/// A pull-based chunk provider: `Ok(None)` marks end of stream.
pub(crate) trait ChunkSource {
  /// Fetches the next chunk, if any.
  fn next_chunk(&mut self) -> impl Future<Output = AnyResult<Option<Bytes>>> + Send;
}

impl ChunkSource for reqwest::Response {
  async fn next_chunk(&mut self) -> AnyResult<Option<Bytes>> {
    Ok(self.chunk().await?)
  }
}

/// A chunk destination (file, sink adapter, …).
pub(crate) trait ChunkWriter {
  /// Persists one chunk.
  fn write_chunk(&mut self, chunk: &[u8]) -> impl Future<Output = AnyResult<()>> + Send;
}

impl ChunkWriter for tokio::fs::File {
  async fn write_chunk(&mut self, chunk: &[u8]) -> AnyResult<()> {
    use tokio::io::AsyncWriteExt;
    self.write_all(chunk).await?;
    Ok(())
  }
}

/// Any [`StreamSink`](super::sink::StreamSink) is directly usable as the
/// pipeline's writer — the 2.2 and 3.0 sink contracts are the same shape.
impl<S: super::sink::StreamSink> ChunkWriter for S {
  async fn write_chunk(&mut self, chunk: &[u8]) -> AnyResult<()> {
    super::sink::StreamSink::write_chunk(self, chunk).await
  }
}

/// Everything the pump needs besides the chunk flow itself.
pub(crate) struct PumpSpec<'a> {
  /// Hooks receiving `ChunkWritten` / `Progress` events.
  pub hooks: &'a [Arc<dyn DownloadHook>],
  /// Cooperative cancellation flag.
  pub cancel: Option<&'a Arc<AtomicBool>>,
  /// Average-speed cap in bytes per second.
  pub max_speed: Option<u64>,
  /// Total size when known (carried in `Progress` events).
  pub total: Option<u64>,
  /// Bytes already present before this pump started (resume offset).
  pub start_offset: u64,
  /// Optional progress bar to advance.
  pub progress_bar: Option<&'a ProgressBar>,
}

impl<'a> PumpSpec<'a> {
  /// Builds a spec from [`DownloadOptions`] plus per-call extras.
  pub fn new(
    options: &'a DownloadOptions,
    total: Option<u64>,
    start_offset: u64,
    progress_bar: Option<&'a ProgressBar>,
  ) -> Self {
    Self {
      hooks: &options.hooks,
      cancel: options.cancel.as_ref(),
      max_speed: options.max_speed,
      total,
      start_offset,
      progress_bar,
    }
  }
}

/// What a pump run produced.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PumpOutcome {
  /// Bytes written by this pump (excludes [`PumpSpec::start_offset`]).
  pub written: u64,
  /// True when the cancel flag stopped the loop early.
  pub cancelled: bool,
}

/// Drives chunks from `source` into `writer`, applying events, progress
/// throttling, the speed cap, cancellation, and progress-bar updates.
///
/// # Errors
///
/// Propagates errors from either side; a hook error also aborts the pump.
pub(crate) async fn pump<S, W>(
  source: &mut S,
  writer: &mut W,
  spec: PumpSpec<'_>,
) -> AnyResult<PumpOutcome>
where
  S: ChunkSource,
  W: ChunkWriter,
{
  let started = Instant::now();
  let mut last_progress: Option<Instant> = None;
  let mut written: u64 = 0;

  while let Some(chunk) = source.next_chunk().await? {
    if spec.cancel.is_some_and(|f| f.load(Ordering::Relaxed)) {
      info!(
        "transfer cancelled after {} bytes",
        spec.start_offset + written
      );
      return Ok(PumpOutcome {
        written,
        cancelled: true,
      });
    }

    let offset = spec.start_offset + written;
    writer.write_chunk(&chunk).await?;
    written += chunk.len() as u64;

    super::dispatch_hooks(
      spec.hooks,
      &DownloadEvent::ChunkWritten {
        offset,
        len: chunk.len(),
      },
    )?;

    // Coarse progress events, rate-limited so slow hooks and log spam
    // cannot fire once per chunk.
    let now = Instant::now();
    let due = last_progress.is_none_or(|t| now.duration_since(t) >= events::PROGRESS_INTERVAL);
    if due {
      super::dispatch_hooks(
        spec.hooks,
        &DownloadEvent::Progress {
          downloaded: spec.start_offset + written,
          total: spec.total,
        },
      )?;
      last_progress = Some(now);
    }

    // Average-speed cap: if we are ahead of the allowed pace, sleep the
    // difference so the average never exceeds `max_speed`.
    if let Some(max_speed) = spec.max_speed
      && max_speed > 0
    {
      // f64 is exact for byte counts below 2^53 — far beyond any real
      // download — so the precision-loss casts are safe here.
      #[allow(clippy::cast_precision_loss)]
      let allowed_elapsed = written as f64 / max_speed as f64;
      let ahead_secs = allowed_elapsed - started.elapsed().as_secs_f64();
      if ahead_secs > 0.0 {
        sleep(Duration::from_secs_f64(ahead_secs)).await;
      }
    }

    if let Some(pb) = spec.progress_bar {
      pb.inc(chunk.len() as u64);
    }
  }

  Ok(PumpOutcome {
    written,
    cancelled: false,
  })
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::download::events::RecordingHook;

  /// In-memory source for tests.
  struct VecSource {
    chunks: std::collections::VecDeque<Bytes>,
  }

  impl ChunkSource for VecSource {
    async fn next_chunk(&mut self) -> AnyResult<Option<Bytes>> {
      Ok(self.chunks.pop_front())
    }
  }

  fn spec<'a>(
    hooks: &'a [Arc<dyn DownloadHook>],
    cancel: Option<&'a Arc<AtomicBool>>,
  ) -> PumpSpec<'a> {
    PumpSpec {
      hooks,
      cancel,
      max_speed: None,
      total: Some(7),
      start_offset: 100,
      progress_bar: None,
    }
  }

  #[tokio::test]
  async fn test_pump_forwards_all_chunks_and_events() -> AnyResult<()> {
    let mut source = VecSource {
      chunks: vec![Bytes::from_static(b"hello"), Bytes::from_static(b"!!")].into(),
    };
    let recorder = Arc::new(RecordingHook::new());
    let hooks: Vec<Arc<dyn DownloadHook>> = vec![recorder.clone()];
    let mut written: Vec<u8> = Vec::new();

    let outcome = pump(
      &mut source,
      &mut MemWriter(&mut written),
      spec(&hooks, None),
    )
    .await?;

    assert_eq!(
      PumpOutcome {
        written: 7,
        cancelled: false
      },
      outcome
    );
    assert_eq!(b"hello!!".as_slice(), written.as_slice());

    let events = recorder.events();
    assert!(events.iter().any(|e| e == "chunk:5"), "{events:?}");
    assert!(events.iter().any(|e| e == "chunk:2"), "{events:?}");
    // Offsets honour start_offset: the first chunk starts at 100 and the
    // first (unthrottled) progress event lands at 105.
    assert!(
      events.iter().any(|e| e == "progress:105"),
      "progress carries absolute position: {events:?}"
    );
    Ok(())
  }

  struct MemWriter<'a>(&'a mut Vec<u8>);

  impl ChunkWriter for MemWriter<'_> {
    async fn write_chunk(&mut self, chunk: &[u8]) -> AnyResult<()> {
      self.0.extend_from_slice(chunk);
      Ok(())
    }
  }

  /// Writer that flips a cancel flag after `flip_after` writes.
  struct FlippingWriter<'a> {
    buf: &'a mut Vec<u8>,
    flag: Arc<AtomicBool>,
    flip_after: usize,
    writes: usize,
  }

  impl ChunkWriter for FlippingWriter<'_> {
    async fn write_chunk(&mut self, chunk: &[u8]) -> AnyResult<()> {
      self.buf.extend_from_slice(chunk);
      self.writes += 1;
      if self.writes == self.flip_after {
        self.flag.store(true, Ordering::Relaxed);
      }
      Ok(())
    }
  }

  #[tokio::test]
  async fn test_pump_stops_on_cancel() -> AnyResult<()> {
    let mut source = VecSource {
      chunks: (0..10).map(|_| Bytes::from_static(b"xx")).collect(),
    };
    let flag = Arc::new(AtomicBool::new(false));
    let hooks: Vec<Arc<dyn DownloadHook>> = Vec::new();
    let flag_for_writer = flag.clone();
    let mut written: Vec<u8> = Vec::new();

    let outcome = pump(
      &mut source,
      &mut FlippingWriter {
        buf: &mut written,
        flag: flag_for_writer,
        flip_after: 2,
        writes: 0,
      },
      spec(&hooks, Some(&flag)),
    )
    .await?;

    // Chunks 1-2 are written; the flag is observed before chunk 3.
    assert!(outcome.cancelled);
    assert_eq!(4, outcome.written);
    assert_eq!(2, written.len() / 2);
    Ok(())
  }

  #[tokio::test]
  async fn test_pump_propagates_writer_errors() {
    struct FailingWriter;
    impl ChunkWriter for FailingWriter {
      async fn write_chunk(&mut self, _chunk: &[u8]) -> AnyResult<()> {
        Err(anyhow::anyhow!("disk full"))
      }
    }

    let mut source = VecSource {
      chunks: vec![Bytes::from_static(b"x")].into(),
    };
    let hooks: Vec<Arc<dyn DownloadHook>> = Vec::new();
    let result = pump(&mut source, &mut FailingWriter, spec(&hooks, None)).await;
    let err = result.expect_err("writer error must abort");
    assert!(err.to_string().contains("disk full"), "{err}");
  }

  #[tokio::test]
  async fn test_pump_hook_error_aborts() -> AnyResult<()> {
    use crate::download::events::FailingHook;

    let mut source = VecSource {
      chunks: vec![Bytes::from_static(b"x")].into(),
    };
    let mut buf = Vec::new();
    let hooks: Vec<Arc<dyn DownloadHook>> = vec![Arc::new(FailingHook::new("stop"))];

    let result = pump(&mut source, &mut MemWriter(&mut buf), spec(&hooks, None)).await;
    assert!(result.is_err());
    Ok(())
  }
}
