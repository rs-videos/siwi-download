//! Integration tests for `Download::stream` and the sink ecosystem
//! (roadmap 2.2, second slice).

use siwi_download::download::events::{DownloadHook, RecordingHook};
use siwi_download::download::sink::{HashSink, MemorySink, TeeSink};
use siwi_download::download::{Download, DownloadOptions};
use siwi_download::error::AnyResult;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use tempfile::tempdir;

/// One-thread HTTP server answering every request with `body` (200).
fn spawn_mock(body: &'static [u8]) -> String {
  let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
  let addr = listener.local_addr().expect("local addr");
  std::thread::spawn(move || {
    for stream in listener.incoming() {
      let Ok(mut stream) = stream else { break };
      let mut buf = [0u8; 4096];
      let _ = stream.read(&mut buf);
      let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
      );
      let _ = stream.write_all(head.as_bytes());
      let _ = stream.write_all(body);
      let _ = stream.flush();
    }
  });
  format!("http://{addr}/file.bin")
}

#[tokio::test]
async fn test_stream_into_memory_sink_without_touching_disk() -> AnyResult<()> {
  let body: &[u8] = &vec![42u8; 50_000];
  let response: &'static [u8] = Box::leak(body.to_vec().into_boxed_slice());
  let url = spawn_mock(response);

  let dir = tempdir()?; // deliberately empty: nothing may be written here
  let mut options = DownloadOptions::default();
  let recorder = Arc::new(RecordingHook::new());
  options.add_hook(recorder.clone() as Arc<dyn DownloadHook>);

  let download = Download::new(dir.path().to_str().unwrap());
  let mut sink = MemorySink::default();
  let report = download.stream(&url, options, &mut sink).await?;

  assert_eq!(report.file_size, Some(50_000));
  assert_eq!(report.range_from, Some(0));
  assert!(
    report.file_path.is_empty(),
    "stream report has no file path"
  );
  assert_eq!(sink.as_slice(), response, "sink must hold the exact body");

  // The storage dir must remain empty: streaming never touches disk.
  assert_eq!(std::fs::read_dir(dir.path())?.count(), 0);

  let events = recorder.events();
  assert_eq!("before_request", events[0]);
  assert_eq!("headers:200", events[1]);
  assert_eq!(
    format!("complete:{}", report.file_path),
    *events.last().expect("terminal event")
  );
  Ok(())
}

#[tokio::test]
async fn test_stream_hash_matches_offline_hash() -> AnyResult<()> {
  let body: &[u8] = b"streaming hash parity check";
  let response: &'static [u8] = Box::leak(body.to_vec().into_boxed_slice());
  let url = spawn_mock(response);

  let dir = tempdir()?;
  let mut sink = HashSink::sha256();
  let download = Download::new(dir.path().to_str().unwrap());
  download
    .stream(&url, DownloadOptions::default(), &mut sink)
    .await?;

  let streaming_digest = sink.hex_digest().expect("digest after finalize");
  // Reference value computed the offline way.
  use sha2::{Digest as _, Sha256};
  let expected = hex::encode(Sha256::digest(body));
  assert_eq!(expected, streaming_digest);
  assert!(sink.matches(&expected));
  Ok(())
}

#[tokio::test]
async fn test_stream_tee_fans_out_to_hash_and_memory() -> AnyResult<()> {
  let body: &[u8] = b"tee fan-out payload";
  let response: &'static [u8] = Box::leak(body.to_vec().into_boxed_slice());
  let url = spawn_mock(response);

  let dir = tempdir()?;
  let mut tee = TeeSink::new(HashSink::sha256(), MemorySink::default());
  let download = Download::new(dir.path().to_str().unwrap());
  download
    .stream(&url, DownloadOptions::default(), &mut tee)
    .await?;

  assert_eq!(tee.second().as_slice(), body);
  use sha2::{Digest as _, Sha256};
  assert!(tee.first().matches(&hex::encode(Sha256::digest(body))));
  Ok(())
}

#[tokio::test]
async fn test_stream_reports_error_status_on_http_error() -> AnyResult<()> {
  // Server answering 500 for everything.
  let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
  let addr = listener.local_addr().expect("addr");
  std::thread::spawn(move || {
    for stream in listener.incoming() {
      let Ok(mut stream) = stream else { break };
      let mut buf = [0u8; 4096];
      let _ = stream.read(&mut buf);
      let _ = stream.write_all(
        b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
      );
    }
  });
  let url = format!("http://{addr}/file.bin");

  let dir = tempdir()?;
  let recorder = Arc::new(RecordingHook::new());
  let mut options = DownloadOptions::default();
  options.add_hook(recorder.clone() as Arc<dyn DownloadHook>);

  let download = Download::new(dir.path().to_str().unwrap());
  let mut sink = MemorySink::default();
  let report = download.stream(&url, options, &mut sink).await?;

  assert_eq!(
    report.download_status,
    Some(siwi_download::download::DownloadStatus::Error)
  );
  assert!(
    recorder
      .events()
      .last()
      .expect("terminal")
      .starts_with("error:"),
    "must end with Error event"
  );
  assert!(sink.as_slice().is_empty());
  Ok(())
}
