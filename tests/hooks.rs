//! Integration tests for the download lifecycle hooks (roadmap 2.2).
//!
//! These spin up a local mock HTTP server on a random port and run the real
//! `Download::download` flow against it, asserting on the events that hooks
//! receive — the end-to-end coverage that unit tests can't provide.

use siwi_download::download::events::{CommandHook, DownloadHook, FailingHook, RecordingHook};
use siwi_download::download::{Download, DownloadOptions};
use siwi_download::error::AnyResult;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use tempfile::tempdir;

/// What the mock server should answer for any request.
struct MockResponse {
  /// Full HTTP response head, e.g. "HTTP/1.1 206 Partial Content\r\n...".
  head: String,
  /// Raw body bytes to send after the head.
  body: Vec<u8>,
}

impl MockResponse {
  fn partial(body: &[u8]) -> Self {
    Self {
      head: format!(
        "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
      ),
      body: body.to_vec(),
    }
  }
}

/// Spawns a one-thread HTTP server answering every request with `response`
/// (per connection). Returns the URL to hit it with, path always `/file.bin`.
fn spawn_mock(response: &'static MockResponse) -> String {
  let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
  let addr = listener.local_addr().expect("local addr");
  std::thread::spawn(move || {
    for stream in listener.incoming() {
      let Ok(mut stream) = stream else { break };
      // Drain the request head (we don't need its contents).
      let mut buf = [0u8; 4096];
      let _ = stream.read(&mut buf);
      let head = response.head.as_bytes();
      let _ = stream.write_all(head);
      let _ = stream.write_all(&response.body);
      let _ = stream.flush();
      // Connection: close — drop ends the response.
    }
  });
  format!("http://{addr}/file.bin")
}

#[tokio::test]
async fn test_hooks_fire_in_order_on_fresh_download() -> AnyResult<()> {
  let body: &[u8] = &vec![7u8; 100_000]; // > one typical chunk, forces multiple chunks
  // Leak into 'static for the server thread; tests are short-lived.
  let response: &'static MockResponse = Box::leak(Box::new(MockResponse::partial(body)));
  let url = spawn_mock(response);

  let dir = tempdir()?;
  let recorder = Arc::new(RecordingHook::new());
  let log = Arc::new(LogHookForTest);

  let mut options = DownloadOptions::default();
  options.set_show_progress(false);
  options
    .add_hook(recorder.clone() as Arc<dyn DownloadHook>)
    .add_hook(log as Arc<dyn DownloadHook>);

  let download = Download::new(dir.path().to_str().unwrap());
  download.auto_create_storage_path().await?;
  let report = download.download(&url, options).await?;

  assert_eq!(report.file_size, Some(100_000));
  let path = dir.path().join("file.bin");
  assert_eq!(std::fs::read(&path)?, body);

  let events = recorder.events();
  assert_eq!("before_request", events[0], "first event: {events:?}");
  assert_eq!("headers:206", events[1], "second event: {events:?}");
  assert!(
    events.iter().any(|e| e.starts_with("chunk:")),
    "expected chunk events: {events:?}"
  );
  assert_eq!(
    format!("complete:{}", path.display()),
    *events.last().expect("terminal event"),
    "last event must be Complete: {events:?}"
  );
  Ok(())
}

/// Cheap stand-in that proves a second hook observes the same stream.
struct LogHookForTest;

impl DownloadHook for LogHookForTest {
  fn on_event(&self, _event: siwi_download::download::DownloadEvent<'_>) -> AnyResult<()> {
    Ok(())
  }
}

#[tokio::test]
async fn test_failing_hook_aborts_download() -> AnyResult<()> {
  let response: &'static MockResponse = Box::leak(Box::new(MockResponse::partial(b"hello")));
  let url = spawn_mock(response);

  let dir = tempdir()?;
  let mut options = DownloadOptions::default();
  options.add_hook(Arc::new(FailingHook::new("stop-the-download")));

  let download = Download::new(dir.path().to_str().unwrap());
  download.auto_create_storage_path().await?;
  let err = download
    .download(&url, options)
    .await
    .expect_err("hook failure must abort the download");
  assert!(
    err.to_string().contains("stop-the-download"),
    "unexpected error: {err}"
  );
  Ok(())
}

#[tokio::test]
async fn test_command_hook_runs_on_complete() -> AnyResult<()> {
  let response: &'static MockResponse = Box::leak(Box::new(MockResponse::partial(b"hello world")));
  let url = spawn_mock(response);

  let dir = tempdir()?;
  let marker = dir.path().join("marker.txt");
  // The command writes a marker using the env vars the hook provides.
  let cmd = format!(
    "echo \"$SIWI_FILE_SIZE:$SIWI_STATUS\" > {}",
    marker.display()
  );
  let recorder = Arc::new(RecordingHook::new());

  let mut options = DownloadOptions::default();
  options
    .add_hook(Arc::new(CommandHook::new(cmd)))
    .add_hook(recorder.clone() as Arc<dyn DownloadHook>);

  let download = Download::new(dir.path().to_str().unwrap());
  download.auto_create_storage_path().await?;
  let report = download.download(&url, options).await?;
  assert_eq!(
    report.download_status,
    Some(siwi_download::download::DownloadStatus::Complete)
  );

  let marker_content = std::fs::read_to_string(&marker)?;
  assert_eq!(
    "11:complete\n", marker_content,
    "CommandHook must pass size and status via env"
  );
  Ok(())
}

#[tokio::test]
async fn test_checksum_mismatch_fires_error_event() -> AnyResult<()> {
  let response: &'static MockResponse = Box::leak(Box::new(MockResponse::partial(b"corrupted!")));
  let url = spawn_mock(response);

  let dir = tempdir()?;
  let recorder = Arc::new(RecordingHook::new());

  let mut options = DownloadOptions::default();
  options.set_checksum(siwi_download::download::Algorithm::Sha256, "0".repeat(64));
  options.add_hook(recorder.clone() as Arc<dyn DownloadHook>);

  let download = Download::new(dir.path().to_str().unwrap());
  download.auto_create_storage_path().await?;
  let report = download.download(&url, options).await?;

  assert_eq!(
    report.download_status,
    Some(siwi_download::download::DownloadStatus::Error)
  );
  assert_eq!(report.checksum_verified, Some(false));

  let events = recorder.events();
  assert!(
    events
      .last()
      .expect("terminal event")
      .starts_with("error:checksum mismatch"),
    "last event must be the checksum Error: {events:?}"
  );
  Ok(())
}
