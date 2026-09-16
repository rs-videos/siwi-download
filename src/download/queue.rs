//! Multi-task download orchestration (roadmap 2.3).
//!
//! [`DownloadQueue`] runs a set of [`DownloadTask`]s with a concurrency cap
//! and `depends_on` ordering. Tasks whose dependencies failed are skipped
//! (reported as errored with a reason), and cycle detection rejects
//! contradictory graphs up front.
//!
//! State files persist the task *definitions* (not per-file byte progress);
//! resuming an interrupted batch therefore works through `siwi-download`'s
//! built-in breakpoint continuation: unfinished files continue where they
//! stopped, already-complete files short-circuit via `416`/`Exists`.
//!
//! # Example
//!
//! ```rust,no_run
//! use siwi_download::download::queue::{DownloadQueue, DownloadTask};
//! use siwi_download::download::{Download, DownloadOptions};
//!
//! # async fn example() -> siwi_download::error::AnyResult<()> {
//! let mut queue = DownloadQueue::new(2);
//! queue
//!     .push(DownloadTask::new("model", "https://example.com/model.bin"))
//!     .push(DownloadTask::new("data", "https://example.com/data.tar.gz"));
//! queue.tasks[1].depends_on(["model"]);
//!
//! let download = Download::new("./downloads");
//! let options = DownloadOptions::default();
//! let results = queue.run(&download, &options).await?;
//! # Ok(())
//! # }
//! ```

use crate::{
  download::{Download, DownloadOptions, DownloadReport, DownloadStatus},
  error::AnyResult,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// One unit of work in a [`DownloadQueue`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadTask {
  /// Unique identifier; other tasks reference it in `depends_on`.
  pub id: String,
  /// URL to download.
  pub url: String,
  /// Optional per-task filename override (manifest key: `file_name`).
  #[serde(rename = "file_name", default, skip_serializing_if = "Option::is_none")]
  pub maybe_file_name: Option<String>,
  /// Optional per-task output directory; overrides the queue runner's base
  /// directory when set (manifest key: `output`).
  #[serde(rename = "output", default, skip_serializing_if = "Option::is_none")]
  pub maybe_output: Option<String>,
  /// Task ids that must complete successfully before this one starts.
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub depends_on: Vec<String>,
}

impl DownloadTask {
  /// Creates a task with `id` and `url`, no dependencies.
  pub fn new<S: Into<String>, U: Into<String>>(id: S, url: U) -> Self {
    Self {
      id: id.into(),
      url: url.into(),
      maybe_file_name: None,
      maybe_output: None,
      depends_on: Vec::new(),
    }
  }

  /// Overrides the saved filename.
  #[must_use]
  pub fn file_name<S: Into<String>>(mut self, name: S) -> Self {
    self.maybe_file_name = Some(name.into());
    self
  }

  /// Overrides the output directory for this task.
  #[must_use]
  pub fn output<S: Into<String>>(mut self, dir: S) -> Self {
    self.maybe_output = Some(dir.into());
    self
  }

  /// Declares dependencies on other task ids.
  pub fn depends_on<I, S>(&mut self, ids: I) -> &mut Self
  where
    I: IntoIterator<Item = S>,
    S: Into<String>,
  {
    self.depends_on.extend(ids.into_iter().map(Into::into));
    self
  }
}

/// The outcome for one task after [`DownloadQueue::run`].
#[derive(Debug)]
pub struct TaskResult {
  /// The task id.
  pub task_id: String,
  /// The download report. Skipped tasks carry `DownloadStatus::Error` with
  /// a "skipped" message instead of a real attempt.
  pub report: DownloadReport,
}

impl TaskResult {
  /// True when the task's download finished with `DownloadStatus::Complete`.
  #[must_use]
  pub fn is_success(&self) -> bool {
    self.report.download_status == Some(DownloadStatus::Complete)
  }
}

/// A queued batch of downloads with bounded concurrency and dependency
/// ordering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadQueue {
  /// Tasks in declaration order.
  pub tasks: Vec<DownloadTask>,
  /// Maximum number of tasks downloading at the same time.
  pub max_concurrent: usize,
  /// Where [`DownloadQueue::save_state`] persists the definitions.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub state_file: Option<PathBuf>,
}

impl DownloadQueue {
  /// Creates an empty queue allowing `max_concurrent` parallel downloads.
  ///
  /// # Panics
  ///
  /// Panics when `max_concurrent == 0` — nothing would ever run.
  #[must_use]
  pub fn new(max_concurrent: usize) -> Self {
    assert!(max_concurrent > 0, "max_concurrent must be at least 1");
    Self {
      tasks: Vec::new(),
      max_concurrent,
      state_file: None,
    }
  }

  /// Appends a task.
  ///
  /// # Panics
  ///
  /// Panics when a task with the same `id` already exists (duplicate ids
  /// would make `depends_on` ambiguous).
  pub fn push(&mut self, task: DownloadTask) -> &mut Self {
    assert!(
      !self.tasks.iter().any(|t| t.id == task.id),
      "duplicate task id `{}`",
      task.id
    );
    self.tasks.push(task);
    self
  }

  /// Sets the state file used by [`DownloadQueue::save_state`].
  #[must_use]
  pub fn state_file<P: Into<PathBuf>>(mut self, path: P) -> Self {
    self.state_file = Some(path.into());
    self
  }

  /// Validates the dependency graph: unknown references and cycles are
  /// errors.
  ///
  /// # Errors
  ///
  /// Returns an error listing unknown dependency ids or the detected cycle.
  pub fn validate(&self) -> AnyResult<()> {
    let ids: HashSet<&str> = self.tasks.iter().map(|t| t.id.as_str()).collect();
    for task in &self.tasks {
      for dep in &task.depends_on {
        if !ids.contains(dep.as_str()) {
          return Err(anyhow::anyhow!(
            "task `{}` depends on unknown task `{dep}`",
            task.id
          ));
        }
      }
    }
    if let Some(cycle) = self.find_cycle() {
      return Err(anyhow::anyhow!(
        "dependency cycle detected: {}",
        cycle.join(" -> ")
      ));
    }
    Ok(())
  }

  /// Kahn-style cycle detection; returns one concrete cycle for error
  /// reporting.
  fn find_cycle(&self) -> Option<Vec<String>> {
    let mut indegree: HashMap<&str, usize> = HashMap::new();
    let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();
    for task in &self.tasks {
      indegree.entry(task.id.as_str()).or_insert(0);
      for dep in &task.depends_on {
        *indegree.entry(task.id.as_str()).or_insert(0) += 1;
        dependents
          .entry(dep.as_str())
          .or_default()
          .push(task.id.as_str());
      }
    }
    let mut ready: VecDeque<&str> = indegree
      .iter()
      .filter(|(_, d)| **d == 0)
      .map(|(&id, _)| id)
      .collect();
    let mut processed = 0usize;
    while let Some(id) = ready.pop_front() {
      processed += 1;
      for &next in dependents.get(id).into_iter().flatten() {
        let d = indegree.get_mut(next).expect("task in graph");
        *d -= 1;
        if *d == 0 {
          ready.push_back(next);
        }
      }
    }
    if processed == self.tasks.len() {
      return None;
    }
    // Walk the remaining (cyclic) subgraph to show a concrete cycle.
    let remaining: HashSet<&str> = indegree
      .iter()
      .filter(|(_, d)| **d > 0)
      .map(|(&id, _)| id)
      .collect();
    let start = *remaining.iter().next().expect("cycle non-empty");
    let mut cycle = vec![start.to_owned()];
    let mut cursor: String = start.to_owned();
    loop {
      let task = self.tasks.iter().find(|t| t.id == cursor).expect("task");
      let next = task
        .depends_on
        .iter()
        .find(|d| remaining.contains(d.as_str()))
        .expect("cycle edge");
      if *next == start {
        break;
      }
      cycle.push(next.clone());
      cursor = next.clone();
    }
    cycle.push(start.to_owned());
    Some(cycle)
  }

  /// Runs every task: up to `max_concurrent` at a time, each task starting
  /// only once all of its `depends_on` have completed successfully.
  ///
  /// Tasks whose dependencies failed (or were skipped) are skipped: their
  /// [`TaskResult::report`] carries `DownloadStatus::Error` with a
  /// "skipped" message and no download is attempted.
  ///
  /// Results are returned in task declaration order.
  ///
  /// # Errors
  ///
  /// Returns an error if the graph is invalid (unknown deps / cycle) or a
  /// per-task output directory cannot be created. Individual download
  /// failures are *not* errors — they are reported per task.
  // The scheduler is one cohesive state loop (propagate skips -> fill
  // capacity -> await); splitting it would scatter the invariants.
  #[allow(clippy::too_many_lines)]
  pub async fn run(
    &self,
    downloader: &Download,
    base_options: &DownloadOptions,
  ) -> AnyResult<Vec<TaskResult>> {
    self.validate()?;

    let mut results: Vec<Option<TaskResult>> = (0..self.tasks.len()).map(|_| None).collect();
    let mut done: HashSet<String> = HashSet::new();
    let mut failed: HashSet<String> = HashSet::new();
    let mut remaining: VecDeque<usize> = (0..self.tasks.len()).collect();
    let mut join_set: tokio::task::JoinSet<(usize, AnyResult<DownloadReport>)> =
      tokio::task::JoinSet::new();

    while !remaining.is_empty() || !join_set.is_empty() {
      // 1) Propagate skipped tasks until stable: any task whose dependency
      //    failed (or was itself skipped) is marked skipped without a
      //    download attempt.
      loop {
        let mut propagated = false;
        let mut i = 0;
        while i < remaining.len() {
          let idx = remaining[i];
          let task = &self.tasks[idx];
          if let Some(dep) = task.depends_on.iter().find(|d| failed.contains(*d)) {
            let dep = dep.clone();
            warn!(task = %task.id, dependency = %dep, "queue task skipped");
            let mut report = DownloadReport::new(
              task.url.clone(),
              task.maybe_file_name.clone().unwrap_or_default(),
              String::new(),
              task
                .maybe_output
                .clone()
                .unwrap_or_else(|| downloader.storage_path.clone()),
              String::new(),
            );
            report
              .set_download_status(DownloadStatus::Error)
              .set_msg(format!("skipped: dependency `{dep}` failed"));
            results[idx] = Some(TaskResult {
              task_id: task.id.clone(),
              report,
            });
            failed.insert(task.id.clone());
            remaining.remove(i);
            propagated = true;
          } else {
            i += 1;
          }
        }
        if !propagated {
          break;
        }
      }

      // 2) Fill the concurrency budget with runnable tasks.
      while join_set.len() < self.max_concurrent {
        let Some(pos) = remaining
          .iter()
          .position(|&idx| self.tasks[idx].depends_on.iter().all(|d| done.contains(d)))
        else {
          break;
        };
        let idx = remaining.remove(pos).expect("position from len check");
        let task = self.tasks[idx].clone();
        let dl = match task.maybe_output.as_ref() {
          Some(dir) => Download::new(dir.clone()),
          None => Download::new(downloader.storage_path.clone()),
        };
        if task.maybe_output.is_some() {
          dl.auto_create_storage_path().await?;
        }
        let opts = DownloadOptions {
          maybe_file_name: task.maybe_file_name.clone(),
          maybe_headers: base_options.maybe_headers.clone(),
          maybe_proxy: base_options.maybe_proxy.clone(),
          show_progress: base_options.show_progress,
          maybe_checksum: base_options.maybe_checksum.clone(),
          max_speed: base_options.max_speed,
          maybe_if_modified_since: base_options.maybe_if_modified_since,
          maybe_if_none_match: base_options.maybe_if_none_match.clone(),
          hooks: base_options.hooks.clone(),
        };
        join_set.spawn(async move {
          let report = dl.download(&task.url, opts).await;
          (idx, report)
        });
      }

      // 3) Nothing running and nothing runnable: for a validated DAG this
      //    means every remaining task has run. Defensive break otherwise.
      if join_set.is_empty() {
        break;
      }

      // 4) Wait for one task to finish, record its outcome.
      let Some(joined) = join_set.join_next().await else {
        break;
      };
      let (idx, joined) = joined.map_err(|e| anyhow::anyhow!("queue task panicked: {e}"))?;
      // A per-task download error (network failure etc.) fails only that
      // task, not the whole queue.
      let report = joined.unwrap_or_else(|e| {
        let mut r = DownloadReport::new(
          self.tasks[idx].url.clone(),
          self.tasks[idx].maybe_file_name.clone().unwrap_or_default(),
          String::new(),
          self.tasks[idx]
            .maybe_output
            .clone()
            .unwrap_or_else(|| downloader.storage_path.clone()),
          String::new(),
        );
        r.set_download_status(DownloadStatus::Error)
          .set_msg(format!("download error: {e}"));
        r
      });
      let task_id = self.tasks[idx].id.clone();
      let ok = report.download_status == Some(DownloadStatus::Complete);
      if ok {
        info!(task = %task_id, "queue task complete");
        done.insert(task_id);
      } else {
        warn!(task = %task_id, "queue task failed");
        failed.insert(task_id);
      }
      results[idx] = Some(TaskResult {
        task_id: self.tasks[idx].id.clone(),
        report,
      });
    }

    Ok(results.into_iter().flatten().collect())
  }

  /// Persists the task definitions as JSON so an interrupted batch can be
  /// re-loaded and re-run (unfinished files resume via breakpoint
  /// continuation).
  ///
  /// # Errors
  ///
  /// Returns an error when the state file cannot be written.
  pub fn save_state(&self, path: &Path) -> AnyResult<()> {
    if let Some(parent) = path.parent()
      && !parent.as_os_str().is_empty()
    {
      std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(self)?;
    std::fs::write(path, json)?;
    info!(
      path = %path.display(),
      tasks = self.tasks.len(),
      "queue state saved"
    );
    Ok(())
  }

  /// Loads a queue previously written by [`DownloadQueue::save_state`].
  ///
  /// # Errors
  ///
  /// Returns an error when the file cannot be read or parsed.
  pub fn load_state(path: &Path) -> AnyResult<Self> {
    let json = std::fs::read_to_string(path)?;
    let queue: DownloadQueue = serde_json::from_str(&json)?;
    Ok(queue)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_new_queue_rejects_zero_concurrency() {
    let result = std::panic::catch_unwind(|| DownloadQueue::new(0));
    assert!(result.is_err());
  }

  #[test]
  fn test_push_and_duplicate_id() {
    let mut q = DownloadQueue::new(2);
    q.push(DownloadTask::new("a", "https://x/a"));
    let result = std::panic::catch_unwind(move || {
      q.push(DownloadTask::new("a", "https://x/dup"));
    });
    assert!(result.is_err());
  }

  #[test]
  fn test_validate_unknown_dependency() {
    let mut q = DownloadQueue::new(1);
    q.push(DownloadTask::new("a", "https://x/a"));
    let mut t = DownloadTask::new("b", "https://x/b");
    t.depends_on(["ghost"]);
    q.push(t);
    let err = q.validate().expect_err("unknown dep must fail");
    assert!(err.to_string().contains("unknown task `ghost`"), "{err}");
  }

  #[test]
  fn test_validate_detects_cycle() {
    let mut q = DownloadQueue::new(1);
    let mut a = DownloadTask::new("a", "https://x/a");
    a.depends_on(["b"]);
    let mut b = DownloadTask::new("b", "https://x/b");
    b.depends_on(["a"]);
    q.push(a);
    q.push(b);
    let err = q.validate().expect_err("cycle must fail");
    assert!(err.to_string().contains("cycle"), "{err}");
  }

  #[test]
  fn test_validate_accepts_diamond() {
    let mut q = DownloadQueue::new(4);
    q.push(DownloadTask::new("root", "https://x/root"));
    let mut left = DownloadTask::new("left", "https://x/left");
    left.depends_on(["root"]);
    let mut right = DownloadTask::new("right", "https://x/right");
    right.depends_on(["root"]);
    let mut join = DownloadTask::new("join", "https://x/join");
    join.depends_on(["left", "right"]);
    q.push(left);
    q.push(right);
    q.push(join);
    assert!(q.validate().is_ok());
  }

  #[test]
  fn test_task_builder_fluent() {
    let t = DownloadTask::new("a", "https://x/a")
      .file_name("a.bin")
      .output("./out");
    assert_eq!(t.maybe_file_name.as_deref(), Some("a.bin"));
    assert_eq!(t.maybe_output.as_deref(), Some("./out"));
    assert!(t.depends_on.is_empty());
  }

  #[test]
  fn test_state_round_trip() -> AnyResult<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("nested").join("state.json");
    let mut q = DownloadQueue::new(3).state_file(&path);
    q.push(DownloadTask::new("a", "https://x/a").file_name("a.bin"));
    let mut b = DownloadTask::new("b", "https://x/b");
    b.depends_on(["a"]);
    q.push(b);
    q.save_state(&path)?;

    let loaded = DownloadQueue::load_state(&path)?;
    assert_eq!(loaded.max_concurrent, 3);
    assert_eq!(loaded.tasks.len(), 2);
    assert_eq!(loaded.tasks[0].id, "a");
    assert_eq!(loaded.tasks[0].maybe_file_name.as_deref(), Some("a.bin"));
    assert_eq!(loaded.tasks[1].depends_on, vec!["a".to_owned()]);
    Ok(())
  }
}
