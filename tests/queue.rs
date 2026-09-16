//! Integration tests for `DownloadQueue` (roadmap 2.3): dependency
//! ordering, failure propagation, and concurrency capping against a real
//! (mock) HTTP server.

use siwi_download::download::queue::{DownloadQueue, DownloadTask};
use siwi_download::download::{Download, DownloadOptions, DownloadStatus};
use siwi_download::error::AnyResult;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::tempdir;

/// Mock server serving `/a`, `/b`, `/c`, `/fail` with per-path bodies.
/// `/fail` answers 500. Tracks peak concurrent in-flight GETs.
struct MockServer {
  url_for: HashMap<&'static str, String>,
  peak: Arc<AtomicUsize>,
}

impl MockServer {
  fn spawn() -> Self {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let peak = Arc::new(AtomicUsize::new(0));
    let in_flight = Arc::new(AtomicUsize::new(0));
    let peak_clone = peak.clone();
    let in_flight_clone = in_flight.clone();

    std::thread::spawn(move || {
      for stream in listener.incoming() {
        let Ok(mut stream) = stream else { break };
        let peak = peak_clone.clone();
        let in_flight = in_flight_clone.clone();
        std::thread::spawn(move || {
          let mut buf = [0u8; 4096];
          let _ = stream.read(&mut buf);
          let request = String::from_utf8_lossy(&buf);
          let path = request
            .split_whitespace()
            .nth(1)
            .and_then(|p| p.rsplit('/').next())
            .unwrap_or("")
            .to_owned();

          let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
          peak.fetch_max(now, Ordering::SeqCst);
          std::thread::sleep(std::time::Duration::from_millis(50));
          in_flight.fetch_sub(1, Ordering::SeqCst);

          let (status, body): (&str, Vec<u8>) = if path == "fail" {
            ("500 Internal Server Error", b"boom".to_vec())
          } else {
            ("200 OK", format!("body-of-{path}").into_bytes())
          };
          let head = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
          );
          let _ = stream.write_all(head.as_bytes());
          let _ = stream.write_all(&body);
          let _ = stream.flush();
        });
      }
    });

    Self {
      url_for: ["a", "b", "c", "fail"]
        .into_iter()
        .map(|p| (p, format!("http://{addr}/{p}")))
        .collect(),
      peak,
    }
  }

  fn url(&self, path: &str) -> String {
    self.url_for[path].clone()
  }
}

#[tokio::test]
async fn test_queue_respects_dependency_order() -> AnyResult<()> {
  let server = MockServer::spawn();
  let dir = tempdir()?;

  let mut queue = DownloadQueue::new(4);
  // `second` must wait for `first`; `third` for `second`.
  queue.push(DownloadTask::new("first", server.url("a")));
  let mut second = DownloadTask::new("second", server.url("b"));
  second.depends_on(["first"]);
  queue.push(second);
  let mut third = DownloadTask::new("third", server.url("c"));
  third.depends_on(["second"]);
  queue.push(third);

  let download = Download::new(dir.path().to_str().unwrap());
  let base_options = DownloadOptions::default();
  let results = queue.run(&download, &base_options).await?;

  assert_eq!(results.len(), 3);
  assert!(results.iter().all(|r| r.is_success()), "{results:?}");

  // Filenames come from the URL tails: /a -> a, /b -> b, /c -> c.
  assert_eq!(std::fs::read_to_string(dir.path().join("a"))?, "body-of-a");
  assert_eq!(std::fs::read_to_string(dir.path().join("b"))?, "body-of-b");
  assert_eq!(std::fs::read_to_string(dir.path().join("c"))?, "body-of-c");
  Ok(())
}

#[tokio::test]
async fn test_queue_failure_propagates_to_dependents() -> AnyResult<()> {
  let server = MockServer::spawn();
  let dir = tempdir()?;

  let mut queue = DownloadQueue::new(2);
  queue.push(DownloadTask::new("broken", server.url("fail")));
  let mut child = DownloadTask::new("child", server.url("a"));
  child.depends_on(["broken"]);
  queue.push(child);
  let mut grandchild = DownloadTask::new("grandchild", server.url("b"));
  grandchild.depends_on(["child"]);
  queue.push(grandchild);
  // Independent task must still run despite the failure elsewhere.
  queue.push(DownloadTask::new("independent", server.url("c")));

  let download = Download::new(dir.path().to_str().unwrap());
  let base_options = DownloadOptions::default();
  let results = queue.run(&download, &base_options).await?;

  let by_id: HashMap<&str, &siwi_download::download::TaskResult> =
    results.iter().map(|r| (r.task_id.as_str(), r)).collect();

  assert_eq!(
    by_id["broken"].report.download_status,
    Some(DownloadStatus::Error),
    "the failing task is errored"
  );
  let child = &by_id["child"];
  assert_eq!(child.report.download_status, Some(DownloadStatus::Error));
  assert!(
    child
      .report
      .msg
      .as_deref()
      .is_some_and(|m| m.contains("skipped: dependency `broken`")),
    "child must be skipped with a reason: {:?}",
    child.report.msg
  );
  assert_eq!(
    by_id["grandchild"].report.download_status,
    Some(DownloadStatus::Error),
    "skips cascade transitively"
  );
  assert!(
    by_id["independent"].is_success(),
    "unrelated tasks still succeed"
  );
  Ok(())
}

#[tokio::test]
async fn test_queue_caps_concurrency() -> AnyResult<()> {
  let server = MockServer::spawn();
  let dir = tempdir()?;

  let mut queue = DownloadQueue::new(1); // strictly serial
  for id in ["a", "b", "c"] {
    queue.push(DownloadTask::new(id, server.url(id)));
  }

  let download = Download::new(dir.path().to_str().unwrap());
  let base_options = DownloadOptions::default();
  let results = queue.run(&download, &base_options).await?;
  assert!(results.iter().all(|r| r.is_success()));
  assert_eq!(
    server.peak.load(Ordering::SeqCst),
    1,
    "max_concurrent=1 must never overlap downloads"
  );
  Ok(())
}

#[tokio::test]
async fn test_queue_runs_independent_tasks_concurrently() -> AnyResult<()> {
  let server = MockServer::spawn();
  let dir = tempdir()?;

  let mut queue = DownloadQueue::new(3);
  for id in ["a", "b", "c"] {
    queue.push(DownloadTask::new(id, server.url(id)));
  }

  let download = Download::new(dir.path().to_str().unwrap());
  let base_options = DownloadOptions::default();
  let results = queue.run(&download, &base_options).await?;
  assert!(results.iter().all(|r| r.is_success()));
  assert!(
    server.peak.load(Ordering::SeqCst) >= 2,
    "independent tasks should overlap when capacity allows"
  );
  Ok(())
}

#[tokio::test]
async fn test_queue_per_task_output_and_filename() -> AnyResult<()> {
  let server = MockServer::spawn();
  let dir = tempdir()?;

  let mut queue = DownloadQueue::new(1);
  queue.push(
    DownloadTask::new("custom", server.url("a"))
      .output(dir.path().join("sub").to_str().unwrap())
      .file_name("renamed.bin"),
  );

  let download = Download::new(dir.path().to_str().unwrap());
  let base_options = DownloadOptions::default();
  let results = queue.run(&download, &base_options).await?;
  assert!(results[0].is_success());

  let written = std::fs::read(dir.path().join("sub").join("renamed.bin"))?;
  assert_eq!(written, b"body-of-a");
  Ok(())
}
