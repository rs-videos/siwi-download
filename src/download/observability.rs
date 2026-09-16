//! Observability primitives: metrics and access logging (roadmap 2.4).
//!
//! Both are `DownloadHook` implementations,
//! so they compose with the normal hook registration — no special wiring:
//!
//! ```rust,no_run
//! use std::sync::Arc;
//! use siwi_download::download::observability::{AccessLogHook, Metrics};
//! use siwi_download::download::DownloadOptions;
//!
//! # fn example() -> siwi_download::error::AnyResult<()> {
//! let metrics = Arc::new(Metrics::default());
//! let mut options = DownloadOptions::default();
//! options
//!     .add_hook(metrics.clone())
//!     .add_hook(Arc::new(AccessLogHook::to_file("./access.jsonl")?));
//!
//! // after downloads: metrics.render_prometheus() yields Prometheus text
//! # Ok(())
//! # }
//! ```

use super::events::{DownloadEvent, DownloadHook};
use crate::error::AnyResult;
use serde::Serialize;
use std::fmt::Write as _;
use std::fs::File;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::warn;

/// Cumulative download statistics, Prometheus-flavored.
///
/// Counters are lock-free atomics; one instance can be shared across any
/// number of concurrent downloads via [`std::sync::Arc`].
#[derive(Debug, Default)]
pub struct Metrics {
  /// Downloads that reached `DownloadStatus::Complete`.
  downloads_complete: AtomicU64,
  /// Downloads that ended in any other terminal state (`Error`, `Exists`).
  downloads_not_complete: AtomicU64,
  /// Total bytes written across all downloads (excludes resumed prefixes).
  bytes_written_total: AtomicU64,
  /// Total bytes skipped thanks to breakpoint continuation.
  resume_bytes_total: AtomicU64,
  /// Probe retries performed.
  retries_total: AtomicU64,
  /// Cumulative wall-clock download time in milliseconds.
  duration_ms_total: AtomicU64,
}

impl Metrics {
  /// A fresh, zeroed metrics instance.
  #[must_use]
  pub fn new() -> Self {
    Self::default()
  }

  /// Number of fully completed downloads.
  #[must_use]
  pub fn downloads_complete(&self) -> u64 {
    self.downloads_complete.load(Ordering::Relaxed)
  }

  /// Number of downloads that ended in a non-complete terminal state.
  #[must_use]
  pub fn downloads_not_complete(&self) -> u64 {
    self.downloads_not_complete.load(Ordering::Relaxed)
  }

  /// Total bytes written (resumed prefixes excluded).
  #[must_use]
  pub fn bytes_written_total(&self) -> u64 {
    self.bytes_written_total.load(Ordering::Relaxed)
  }

  /// Total bytes skipped by breakpoint continuation.
  #[must_use]
  pub fn resume_bytes_total(&self) -> u64 {
    self.resume_bytes_total.load(Ordering::Relaxed)
  }

  /// Total probe retries.
  #[must_use]
  pub fn retries_total(&self) -> u64 {
    self.retries_total.load(Ordering::Relaxed)
  }

  /// Cumulative download duration in milliseconds.
  #[must_use]
  pub fn duration_ms_total(&self) -> u64 {
    self.duration_ms_total.load(Ordering::Relaxed)
  }

  /// Renders the counters in Prometheus text exposition format.
  #[must_use]
  pub fn render_prometheus(&self) -> String {
    let mut out = String::new();
    let _ = writeln!(
      out,
      "# HELP siwi_downloads_total Downloads by terminal status."
    );
    let _ = writeln!(out, "# TYPE siwi_downloads_total counter");
    let _ = writeln!(
      out,
      "siwi_downloads_total{{status=\"complete\"}} {}",
      self.downloads_complete()
    );
    let _ = writeln!(
      out,
      "siwi_downloads_total{{status=\"not_complete\"}} {}",
      self.downloads_not_complete()
    );
    let _ = writeln!(
      out,
      "# HELP siwi_download_bytes_total Bytes written to sinks."
    );
    let _ = writeln!(out, "# TYPE siwi_download_bytes_total counter");
    let _ = writeln!(
      out,
      "siwi_download_bytes_total {}",
      self.bytes_written_total()
    );
    let _ = writeln!(
      out,
      "# HELP siwi_download_resume_bytes_total Bytes skipped by breakpoint continuation."
    );
    let _ = writeln!(out, "# TYPE siwi_download_resume_bytes_total counter");
    let _ = writeln!(
      out,
      "siwi_download_resume_bytes_total {}",
      self.resume_bytes_total()
    );
    let _ = writeln!(
      out,
      "# HELP siwi_download_retries_total Probe retries performed."
    );
    let _ = writeln!(out, "# TYPE siwi_download_retries_total counter");
    let _ = writeln!(out, "siwi_download_retries_total {}", self.retries_total());
    let _ = writeln!(
      out,
      "# HELP siwi_download_duration_ms_total Cumulative download wall-clock time in milliseconds."
    );
    let _ = writeln!(out, "# TYPE siwi_download_duration_ms_total counter");
    let _ = writeln!(
      out,
      "siwi_download_duration_ms_total {}",
      self.duration_ms_total()
    );
    out
  }
}

impl DownloadHook for Metrics {
  fn on_event(&self, event: DownloadEvent<'_>) -> AnyResult<()> {
    match &event {
      DownloadEvent::Retry { .. } => {
        self.retries_total.fetch_add(1, Ordering::Relaxed);
      }
      DownloadEvent::ChunkWritten { len, .. } => {
        self
          .bytes_written_total
          .fetch_add(*len as u64, Ordering::Relaxed);
      }
      DownloadEvent::Complete { report } => {
        self.downloads_complete.fetch_add(1, Ordering::Relaxed);
        if let Some(resumed) = report.range_from {
          self
            .resume_bytes_total
            .fetch_add(resumed, Ordering::Relaxed);
        }
        if let Some(secs) = report.time_used {
          self.duration_ms_total.fetch_add(
            u64::try_from(secs.max(0)).unwrap_or(0) * 1000,
            Ordering::Relaxed,
          );
        }
      }
      DownloadEvent::Error { .. } => {
        self.downloads_not_complete.fetch_add(1, Ordering::Relaxed);
      }
      _ => {}
    }
    Ok(())
  }
}

/// Where [`AccessLogHook`] writes its JSON lines.
#[derive(Debug)]
enum LogTarget {
  Stderr,
  File(Mutex<File>),
}

/// A [`DownloadHook`] appending one JSON line per terminal download outcome
/// (one line per access — hence the name).
///
/// Line schema:
///
/// ```json
/// {"ts":"2026-09-16T12:00:00Z","url":"…","file_path":"…","status":"Complete","size":123,"resumed":0,"time_used":5,"msg":null}
/// ```
///
/// Lines are appended when the file target is used; failures to write are
/// logged via `tracing` and otherwise ignored — an observability hook must
/// never take the download down.
#[derive(Debug)]
pub struct AccessLogHook {
  target: LogTarget,
}

impl AccessLogHook {
  /// Writes access lines to stderr.
  #[must_use]
  pub fn to_stderr() -> Self {
    Self {
      target: LogTarget::Stderr,
    }
  }

  /// Appends access lines to `path` (created on first write).
  ///
  /// # Errors
  ///
  /// Returns an error when the file cannot be opened for appending.
  pub fn to_file(path: impl AsRef<Path>) -> AnyResult<Self> {
    let file = File::options().create(true).append(true).open(path)?;
    Ok(Self {
      target: LogTarget::File(Mutex::new(file)),
    })
  }

  fn write_line(&self, line: &str) {
    match &self.target {
      LogTarget::Stderr => {
        eprintln!("{line}");
      }
      LogTarget::File(file) => {
        let mut file = file.lock().expect("access log lock");
        if let Err(e) = std::io::Write::write_all(&mut *file, format!("{line}\n").as_bytes()) {
          warn!(error = %e, "access log write failed");
        }
      }
    }
  }
}

/// One access-log record, serialized in field order.
#[derive(Debug, Serialize)]
struct AccessRecord<'a> {
  ts: String,
  url: String,
  file_path: String,
  status: String,
  size: Option<u64>,
  resumed: Option<u64>,
  time_used: Option<i64>,
  msg: Option<&'a str>,
}

impl DownloadHook for AccessLogHook {
  fn on_event(&self, event: DownloadEvent<'_>) -> AnyResult<()> {
    // Only terminal outcomes are logged; retry/progress noise stays out.
    let record = match &event {
      DownloadEvent::Complete { report } => AccessRecord {
        ts: crate::utils::date().to_rfc3339(),
        url: report.url.clone(),
        file_path: report.file_path.clone(),
        status: report
          .download_status
          .as_ref()
          .map(|s| format!("{s:?}"))
          .unwrap_or_default(),
        size: report.file_size,
        resumed: report.range_from,
        time_used: report.time_used,
        msg: report.msg.as_deref(),
      },
      DownloadEvent::Error { msg } => AccessRecord {
        ts: crate::utils::date().to_rfc3339(),
        url: String::new(),
        file_path: String::new(),
        status: "Error".to_owned(),
        size: None,
        resumed: None,
        time_used: None,
        msg: Some(msg),
      },
      _ => return Ok(()),
    };
    match serde_json::to_string(&record) {
      Ok(line) => self.write_line(&line),
      Err(e) => warn!(error = %e, "access log serialization failed"),
    }
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::download::DownloadReport;
  use crate::download::events::RecordingHook;

  fn complete_report(size: u64, resumed: u64) -> DownloadReport {
    let mut r = DownloadReport::new("u", "f", "f", "/s", "/s/f");
    r.set_file_size(size)
      .set_range_from(resumed)
      .set_download_start_at()
      .set_download_end_at()
      .gen_time_used()
      .set_download_status(crate::download::DownloadStatus::Complete);
    r
  }

  #[test]
  fn test_metrics_counts_complete_and_bytes() -> AnyResult<()> {
    let m = Metrics::new();
    let r = complete_report(100, 40);
    m.on_event(DownloadEvent::ChunkWritten {
      offset: 40,
      len: 60,
    })?;
    m.on_event(DownloadEvent::Complete { report: &r })?;
    assert_eq!(1, m.downloads_complete());
    assert_eq!(0, m.downloads_not_complete());
    assert_eq!(60, m.bytes_written_total());
    assert_eq!(40, m.resume_bytes_total());
    Ok(())
  }

  #[test]
  fn test_metrics_counts_error_and_retries() -> AnyResult<()> {
    let m = Metrics::new();
    m.on_event(DownloadEvent::Retry {
      attempt: 1,
      status: 503,
    })?;
    m.on_event(DownloadEvent::Error { msg: "x" })?;
    assert_eq!(1, m.retries_total());
    assert_eq!(1, m.downloads_not_complete());
    assert_eq!(0, m.downloads_complete());
    Ok(())
  }

  #[test]
  fn test_metrics_ignores_progress_noise() {
    let m = Metrics::new();
    let _ = m.on_event(DownloadEvent::Progress {
      downloaded: 10,
      total: Some(10),
    });
    assert_eq!(0, m.bytes_written_total(), "progress must not double-count");
  }

  #[test]
  fn test_metrics_renders_prometheus() -> AnyResult<()> {
    let m = Metrics::new();
    let r = complete_report(100, 40);
    m.on_event(DownloadEvent::ChunkWritten {
      offset: 40,
      len: 60,
    })?;
    m.on_event(DownloadEvent::Complete { report: &r })?;
    let text = m.render_prometheus();
    assert!(
      text.contains("siwi_downloads_total{status=\"complete\"} 1"),
      "{text}"
    );
    assert!(text.contains("siwi_download_bytes_total 60"), "{text}");
    assert!(
      text.contains("siwi_download_resume_bytes_total 40"),
      "{text}"
    );
    assert!(text.starts_with("# HELP"), "{text}");
    Ok(())
  }

  #[test]
  fn test_access_log_file_lines() -> AnyResult<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("access.jsonl");
    let hook = AccessLogHook::to_file(&path)?;

    let r = complete_report(100, 0);
    hook.on_event(DownloadEvent::Complete { report: &r })?;
    hook.on_event(DownloadEvent::Error { msg: "boom" })?;
    // Non-terminal events are not logged.
    hook.on_event(DownloadEvent::Progress {
      downloaded: 1,
      total: Some(2),
    })?;

    let content = std::fs::read_to_string(&path)?;
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(2, lines.len(), "exactly two terminal records: {content}");
    let complete_ok = lines[0].contains("\"status\":\"Complete\"");
    let size_ok = lines[0].contains("\"size\":100");
    let error_ok = lines[1].contains("\"status\":\"Error\"");
    assert!(complete_ok, "bad record: {}", lines[0]);
    assert!(size_ok, "bad record: {}", lines[0]);
    assert!(error_ok, "bad record: {}", lines[1]);
    Ok(())
  }

  #[test]
  fn test_recording_hook_still_works_alongside() {
    // Sanity: observability hooks compose with plain hooks.
    let m = Metrics::new();
    let rec = RecordingHook::new();
    let _ = m.on_event(DownloadEvent::Error { msg: "x" });
    let _ = rec.on_event(DownloadEvent::Error { msg: "x" });
    assert_eq!(1, rec.len());
    assert_eq!(1, m.downloads_not_complete());
  }
}
