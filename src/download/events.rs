//! Download lifecycle events and hooks.
//!
//! Hooks let callers observe — and optionally interrupt — a download at
//! well-defined points. Register one via
//! [`DownloadOptions::add_hook`](super::DownloadOptions::add_hook); every
//! registered hook receives every [`DownloadEvent`] in order.
//!
//! A hook returning `Err` aborts the download immediately: the error is
//! propagated out of [`Download::download`](super::Download::download) and
//! no further events are delivered.
//!
//! # Example
//!
//! ```
//! use siwi_download::download::events::{DownloadEvent, DownloadHook};
//! use siwi_download::error::AnyResult;
//! use std::sync::Mutex;
//!
//! struct Counter {
//!     chunks: Mutex<u64>,
//! }
//!
//! impl DownloadHook for Counter {
//!     fn on_event(&self, event: DownloadEvent<'_>) -> AnyResult<()> {
//!         if let DownloadEvent::ChunkWritten { len, .. } = event {
//!             *self.chunks.lock().unwrap() += len as u64;
//!         }
//!         Ok(())
//!     }
//! }
//! ```

use crate::download::DownloadReport;
use crate::error::AnyResult;
use std::fmt::Write as _;
use std::process::Command;
use std::sync::Mutex;
use tracing::{debug, error, info};

/// How often [`DownloadEvent::Progress`] fires at most, per download.
pub(crate) const PROGRESS_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

/// A point in the download lifecycle.
///
/// Events are delivered in this order:
/// `BeforeRequest` → `HeadersReceived` → (`ChunkWritten` | `Progress`)* →
/// `Complete`, or `Error` instead of `Complete` on failure paths.
///
/// Terminal "nothing to do" outcomes (the server answered `304 Not Modified`
/// or `416 Range Not Satisfiable`) end the download after `HeadersReceived`
/// without a terminal event — check the returned report's
/// `download_status` (`Exists`) to detect them.
#[derive(Debug, Clone)]
pub enum DownloadEvent<'a> {
  /// The request is about to be sent (after client construction, before
  /// the probe HEAD request).
  BeforeRequest {
    /// The URL being downloaded.
    url: &'a str,
  },
  /// The probe response headers are in (after retries).
  HeadersReceived {
    /// The URL being downloaded.
    url: &'a str,
    /// Final HTTP status of the probe request.
    status: u16,
    /// Total size in bytes when the server reported one.
    content_length: Option<u64>,
  },
  /// A chunk has been written to the destination.
  ChunkWritten {
    /// Offset of the chunk's first byte within the final file.
    offset: u64,
    /// Chunk size in bytes.
    len: usize,
  },
  /// Coarse-grained progress, rate-limited to at most one event per
  /// 100 ms.
  Progress {
    /// Bytes written so far.
    downloaded: u64,
    /// Total size when known.
    total: Option<u64>,
  },
  /// The download finished successfully. This is the last event.
  Complete {
    /// The final report.
    report: &'a DownloadReport,
  },
  /// The download failed. This is the last event.
  Error {
    /// Human-readable failure description.
    msg: &'a str,
  },
}

/// A callback observing a download's lifecycle.
///
/// Implementations must be thread-safe (`Send + Sync`) and reentrant-safe:
/// hooks are invoked inline on the download task, so long-running work
/// delays the download.
///
/// Returning `Err` aborts the download; the error propagates to the caller
/// of [`Download::download`](crate::download::Download::download).
pub trait DownloadHook: Send + Sync {
  /// Called for every [`DownloadEvent`] of the download.
  ///
  /// # Errors
  ///
  /// Returning an error aborts the download.
  fn on_event(&self, event: DownloadEvent<'_>) -> AnyResult<()>;
}

/// A [`DownloadHook`] that logs every event via `tracing`.
///
/// Handy for debugging: attach it and watch the DEBUG stream.
#[derive(Debug, Default, Clone, Copy)]
pub struct LogHook;

impl DownloadHook for LogHook {
  fn on_event(&self, event: DownloadEvent<'_>) -> AnyResult<()> {
    match &event {
      DownloadEvent::BeforeRequest { url } => debug!(url, "hook: before request"),
      DownloadEvent::HeadersReceived {
        url,
        status,
        content_length,
      } => debug!(url, status, ?content_length, "hook: headers received"),
      DownloadEvent::ChunkWritten { offset, len } => {
        debug!(offset, len, "hook: chunk written");
      }
      DownloadEvent::Progress { downloaded, total } => {
        debug!(downloaded, ?total, "hook: progress");
      }
      DownloadEvent::Complete { report } => info!(
        file_path = %report.file_path,
        status = ?report.download_status,
        "hook: complete"
      ),
      DownloadEvent::Error { msg } => error!(msg, "hook: error"),
    }
    Ok(())
  }
}

/// A [`DownloadHook`] that runs an external command when the download
/// completes.
///
/// The command is executed through the platform shell (`sh -c` on Unix,
/// `cmd /C` on Windows). Context is passed via environment variables —
/// never by string interpolation — so paths with spaces or quotes are safe:
///
/// - `SIWI_FILE_PATH` — final file path
/// - `SIWI_FILE_SIZE` — final size in bytes
/// - `SIWI_URL` — source URL
/// - `SIWI_STATUS` — `complete` or `error` (the event kind)
/// - `SIWI_DOWNLOAD_STATUS` — precise `DownloadStatus` for the operation
///   (e.g. `Some(Complete)`); useful to distinguish a real download from
///   an `Exists` short-circuit
///
/// Note: the command runs inline on the download task; a slow command
/// delays the download's completion. Fire-and-forget semantics (background
/// the process inside your command) are up to the command author.
#[derive(Debug, Clone)]
pub struct CommandHook {
  command: String,
}

impl CommandHook {
  /// Creates a hook that runs `command` (via the platform shell) on
  /// completion.
  #[must_use]
  pub fn new<S: Into<String>>(command: S) -> Self {
    Self {
      command: command.into(),
    }
  }

  /// Path this hook's download will be written to, for tests.
  #[must_use]
  pub fn command(&self) -> &str {
    &self.command
  }
}

impl DownloadHook for CommandHook {
  fn on_event(&self, event: DownloadEvent<'_>) -> AnyResult<()> {
    let (status, report) = match &event {
      DownloadEvent::Complete { report } => ("complete", Some(*report)),
      DownloadEvent::Error { .. } => ("error", None),
      _ => return Ok(()),
    };

    let mut cmd = shell_command(&self.command);
    if let Some(report) = report {
      cmd.env("SIWI_FILE_PATH", report.file_path.as_str());
      cmd.env(
        "SIWI_FILE_SIZE",
        report.file_size.map(|s| s.to_string()).unwrap_or_default(),
      );
      cmd.env("SIWI_URL", report.url.as_str());
      cmd.env(
        "SIWI_DOWNLOAD_STATUS",
        format!("{:?}", report.download_status),
      );
    }
    cmd.env("SIWI_STATUS", status);

    let output = cmd
      .output()
      .map_err(|e| anyhow::anyhow!("hook command `{}` failed to spawn: {e}", self.command))?;
    if !output.status.success() {
      let stderr = String::from_utf8_lossy(&output.stderr);
      let mut msg = format!(
        "hook command `{}` exited with {}",
        self.command, output.status
      );
      if !stderr.trim().is_empty() {
        let _ = write!(msg, ": {}", stderr.trim());
      }
      return Err(anyhow::anyhow!(msg));
    }
    Ok(())
  }
}

/// Builds a shell invocation for `command` on the current platform.
#[cfg(unix)]
fn shell_command(command: &str) -> Command {
  let mut cmd = Command::new("sh");
  cmd.arg("-c").arg(command);
  cmd
}

/// Builds a shell invocation for `command` on the current platform.
#[cfg(windows)]
fn shell_command(command: &str) -> Command {
  let mut cmd = Command::new("cmd");
  cmd.arg("/C").arg(command);
  cmd
}

/// A [`DownloadHook`] collecting every event into a shared vector.
///
/// Intended for tests; kept public so integration tests and downstream
/// users can assert on event streams the same way.
#[derive(Debug, Default)]
pub struct RecordingHook {
  events: Mutex<Vec<String>>,
}

impl RecordingHook {
  /// Creates an empty recorder.
  #[must_use]
  pub fn new() -> Self {
    Self::default()
  }

  /// Snapshot of the recorded event summaries, in delivery order.
  #[must_use]
  pub fn events(&self) -> Vec<String> {
    self.events.lock().expect("recording lock").clone()
  }

  /// Number of events recorded so far.
  #[must_use]
  pub fn len(&self) -> usize {
    self.events.lock().expect("recording lock").len()
  }

  /// True when no events have been recorded.
  #[must_use]
  pub fn is_empty(&self) -> bool {
    self.len() == 0
  }
}

impl DownloadHook for RecordingHook {
  fn on_event(&self, event: DownloadEvent<'_>) -> AnyResult<()> {
    let summary = match &event {
      DownloadEvent::BeforeRequest { .. } => "before_request".to_owned(),
      DownloadEvent::HeadersReceived { status, .. } => format!("headers:{status}"),
      DownloadEvent::ChunkWritten { len, .. } => format!("chunk:{len}"),
      DownloadEvent::Progress { downloaded, .. } => format!("progress:{downloaded}"),
      DownloadEvent::Complete { report } => format!("complete:{}", report.file_path),
      DownloadEvent::Error { msg } => format!("error:{msg}"),
    };
    self.events.lock().expect("recording lock").push(summary);
    Ok(())
  }
}

/// A [`DownloadHook`] that always fails, for testing interruption.
#[derive(Debug)]
pub struct FailingHook {
  message: String,
}

impl FailingHook {
  /// Creates a hook failing with `message` on every event.
  #[must_use]
  pub fn new<S: Into<String>>(message: S) -> Self {
    Self {
      message: message.into(),
    }
  }
}

impl DownloadHook for FailingHook {
  fn on_event(&self, _event: DownloadEvent<'_>) -> AnyResult<()> {
    Err(anyhow::anyhow!("{}", self.message))
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::path::{Path, PathBuf};

  /// Returns the final path component of `path`.
  fn file_name_of(path: &Path) -> Option<PathBuf> {
    path.file_name().map(PathBuf::from)
  }

  #[test]
  fn test_log_hook_never_fails() {
    let hook = LogHook;
    assert!(
      hook
        .on_event(DownloadEvent::BeforeRequest { url: "u" })
        .is_ok()
    );
    assert!(hook.on_event(DownloadEvent::Error { msg: "boom" }).is_ok());
  }

  #[test]
  fn test_command_hook_ignores_non_terminal_events() {
    let hook = CommandHook::new("exit 1"); // would fail if executed
    assert!(
      hook
        .on_event(DownloadEvent::BeforeRequest { url: "u" })
        .is_ok()
    );
    assert!(
      hook
        .on_event(DownloadEvent::ChunkWritten { offset: 0, len: 1 })
        .is_ok()
    );
  }

  #[test]
  fn test_command_hook_success_and_env() -> AnyResult<()> {
    // The command reads env vars the hook is expected to set. On success it
    // exits 0; SIWI_STATUS must be "error" for an Error event.
    let hook = CommandHook::new("test \"$SIWI_STATUS\" = \"error\"");
    hook.on_event(DownloadEvent::Error { msg: "boom" })?;

    let report = DownloadReport::new("u", "f", "f", "/s", "/s/f");
    let hook2 =
      CommandHook::new("test \"$SIWI_STATUS\" = \"complete\" && test -n \"$SIWI_FILE_PATH\"");
    hook2.on_event(DownloadEvent::Complete { report: &report })?;
    Ok(())
  }

  #[test]
  fn test_command_hook_nonzero_exit_is_error() {
    let hook = CommandHook::new("exit 3");
    let err = hook
      .on_event(DownloadEvent::Error { msg: "boom" })
      .expect_err("must fail");
    assert!(err.to_string().contains("exited with"), "{err}");
  }

  #[test]
  fn test_command_hook_unsplittable_command_errors() {
    let hook = CommandHook::new("definitely-not-a-real-binary-xyz");
    assert!(hook.on_event(DownloadEvent::Error { msg: "x" }).is_err());
  }

  #[test]
  fn test_recording_hook_collects_summaries() {
    let hook = RecordingHook::new();
    assert!(hook.is_empty());
    let report = DownloadReport::new("u", "f", "f", "/s", "/s/f");
    let _ = hook.on_event(DownloadEvent::BeforeRequest { url: "u" });
    let _ = hook.on_event(DownloadEvent::HeadersReceived {
      url: "u",
      status: 206,
      content_length: Some(10),
    });
    let _ = hook.on_event(DownloadEvent::ChunkWritten { offset: 0, len: 10 });
    let _ = hook.on_event(DownloadEvent::Complete { report: &report });

    assert_eq!(
      vec![
        "before_request".to_owned(),
        "headers:206".to_owned(),
        "chunk:10".to_owned(),
        "complete:/s/f".to_owned(),
      ],
      hook.events()
    );
    assert_eq!(4, hook.len());
  }

  #[test]
  fn test_failing_hook_always_fails() {
    let hook = FailingHook::new("stop");
    assert!(
      hook
        .on_event(DownloadEvent::BeforeRequest { url: "u" })
        .is_err()
    );
  }

  #[test]
  fn test_file_name_of_helper() {
    assert_eq!(
      Some(PathBuf::from("b.txt")),
      file_name_of(Path::new("/a/b.txt"))
    );
    assert_eq!(None, file_name_of(Path::new("/")));
  }
}
