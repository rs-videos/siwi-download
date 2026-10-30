//! Integration tests for the 3.0 Pipeline (M1, `unstable` feature).

use siwi_download::download::events::{DownloadHook, RecordingHook};
use siwi_download::download::sink::MemorySink;
use siwi_download::error::AnyResult;
use siwi_download::pipeline::{
  FileSource, HttpSource, HttpSourceOptions, Pipeline, PipelineOptions,
};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tempfile::tempdir;

fn spawn_mock(body: &'static [u8]) -> String {
  let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
  let addr = listener.local_addr().expect("addr");
  std::thread::spawn(move || {
    for stream in listener.incoming() {
      let Ok(mut stream) = stream else { break };
      let mut buf = [0u8; 4096];
      let _ = stream.read(&mut buf);
      let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nETag: \"abc123\"\r\nConnection: close\r\n\r\n",
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
async fn test_pipeline_http_to_memory() -> AnyResult<()> {
  let body: &'static [u8] = Box::leak(vec![9u8; 40_000].into_boxed_slice());
  let url = spawn_mock(body);

  let recorder = Arc::new(RecordingHook::new());
  let options = PipelineOptions {
    hooks: vec![recorder.clone() as Arc<dyn DownloadHook>],
    ..PipelineOptions::default()
  };
  let source = HttpSource::new(url, HttpSourceOptions::default())?;
  let report = Pipeline::new(source, MemorySink::default())
    .options(options)
    .run()
    .await?;

  assert_eq!(report.bytes_transferred, 40_000);
  assert_eq!(report.bytes_skipped, 0);
  assert_eq!(report.content_length, Some(40_000));
  assert_eq!(report.etag.as_deref(), Some("\"abc123\""));
  assert!(!report.cancelled);

  let events = recorder.events();
  assert_eq!("before_request", events[0], "{events:?}");
  assert_eq!("headers:200", events[1], "{events:?}");
  assert!(events.iter().any(|e| e.starts_with("chunk:")), "{events:?}");
  assert!(
    events.last().expect("terminal").starts_with("complete:"),
    "{events:?}"
  );
  Ok(())
}

#[tokio::test]
async fn test_pipeline_file_source() -> AnyResult<()> {
  let dir = tempdir()?;
  let path = dir.path().join("input.bin");
  let payload: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
  std::fs::write(&path, &payload)?;

  let report = Pipeline::new(FileSource::new(&path), MemorySink::default())
    .run()
    .await?;

  assert_eq!(report.bytes_transferred, payload.len() as u64);
  assert_eq!(report.content_length, Some(payload.len() as u64));
  assert_eq!(report.status, 0);
  assert!(!report.cancelled);
  Ok(())
}

#[tokio::test]
async fn test_pipeline_honors_cancel() -> AnyResult<()> {
  let body: &'static [u8] = Box::leak(vec![1u8; 100_000].into_boxed_slice());
  let url = spawn_mock(body);

  let flag = Arc::new(AtomicBool::new(true)); // pre-cancelled
  let options = PipelineOptions {
    cancel: Some(flag),
    ..PipelineOptions::default()
  };
  let source = HttpSource::new(url, HttpSourceOptions::default())?;
  let recorder = Arc::new(RecordingHook::new());
  let options = PipelineOptions {
    hooks: vec![recorder.clone() as Arc<dyn DownloadHook>],
    ..options
  };
  let report = Pipeline::new(source, MemorySink::default())
    .options(options)
    .run()
    .await?;

  assert!(report.cancelled);
  assert_eq!(
    report.bytes_transferred, 0,
    "pre-cancelled run writes nothing"
  );
  assert!(
    recorder
      .events()
      .last()
      .expect("terminal")
      .starts_with("error:"),
    "{:?}",
    recorder.events()
  );
  Ok(())
}

#[tokio::test]
async fn test_pipeline_resume_from_offset() -> AnyResult<()> {
  let dir = tempdir()?;
  let path = dir.path().join("resume.bin");
  let payload: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
  std::fs::write(&path, &payload)?;

  let report = Pipeline::new(FileSource::new(&path), MemorySink::default())
    .options(PipelineOptions {
      resume_from: 400,
      ..PipelineOptions::default()
    })
    .run()
    .await?;

  assert_eq!(report.bytes_skipped, 400);
  assert_eq!(report.bytes_transferred, 600);
  assert_eq!(report.content_length, Some(1000));
  Ok(())
}

#[tokio::test]
async fn test_pipeline_http_404_is_error() -> AnyResult<()> {
  let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
  let addr = listener.local_addr().expect("addr");
  std::thread::spawn(move || {
    for stream in listener.incoming() {
      let Ok(mut stream) = stream else { break };
      let mut buf = [0u8; 4096];
      let _ = stream.read(&mut buf);
      let _ = stream
        .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    }
  });
  let url = format!("http://{addr}/missing");

  let source = HttpSource::new(
    url,
    HttpSourceOptions {
      retry_delay: Some(std::time::Duration::from_millis(1)),
      max_retries: Some(1),
      ..HttpSourceOptions::default()
    },
  )?;
  let result = Pipeline::new(source, MemorySink::default()).run().await;
  assert!(result.is_err(), "404 must surface as an error");
  Ok(())
}

#[tokio::test]
async fn test_pipeline_metadata_reports_etag_and_length() -> AnyResult<()> {
  use siwi_download::pipeline::Source;

  let body: &'static [u8] = Box::leak(vec![2u8; 1234].into_boxed_slice());
  let url = spawn_mock(body);
  let source = HttpSource::new(url, HttpSourceOptions::default())?;
  let meta = source.metadata().await?;
  assert_eq!(meta.content_length, Some(1234));
  assert_eq!(meta.status, 200);
  assert_eq!(meta.etag.as_deref(), Some("\"abc123\""));
  Ok(())
}

// ---------------------------------------------------------------------------
// M2: filters
// ---------------------------------------------------------------------------

#[cfg(feature = "gzip")]
#[tokio::test]
async fn test_pipeline_gunzip_filter_decodes_to_sink() -> AnyResult<()> {
  use siwi_download::pipeline::{Filter, GunzipFilter};
  use std::io::Write as _;

  let dir = tempdir()?;
  let plain: Vec<u8> = (0..80_000u32).map(|i| (i % 251) as u8).collect();
  let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
  enc.write_all(&plain)?;
  let gz = enc.finish()?;
  let path = dir.path().join("data.bin.gz");
  std::fs::write(&path, &gz)?;

  // Keep a pointer to the sink's contents after the pipeline consumes it by
  // value: use a shared buffer sink instead of MemorySink.
  struct SharedSink(Arc<std::sync::Mutex<Vec<u8>>>);
  impl siwi_download::download::sink::StreamSink for SharedSink {
    async fn write_chunk(&mut self, chunk: &[u8]) -> AnyResult<()> {
      self.0.lock().unwrap().extend_from_slice(chunk);
      Ok(())
    }
  }

  let shared = Arc::new(std::sync::Mutex::new(Vec::new()));
  let sink = SharedSink(shared.clone());
  let report = Pipeline::new(FileSource::new(&path), sink)
    .filter(Box::new(GunzipFilter::new()) as Box<dyn Filter>)
    .run()
    .await?;

  assert_eq!(
    report.bytes_transferred,
    gz.len() as u64,
    "compressed bytes read"
  );
  assert_eq!(plain, *shared.lock().unwrap(), "sink receives plain bytes");
  Ok(())
}

#[cfg(feature = "gzip")]
#[tokio::test]
async fn test_pipeline_checksum_filter_verifies_plain_payload() -> AnyResult<()> {
  use sha2::{Digest as _, Sha256};
  use siwi_download::pipeline::{ChecksumFilter, Filter, GunzipFilter};
  use std::io::Write as _;

  let dir = tempdir()?;
  let plain = b"filtered checksum payload";
  let expected = hex::encode(Sha256::digest(plain));
  let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
  enc.write_all(plain)?;
  let gz = enc.finish()?;
  let path = dir.path().join("payload.gz");
  std::fs::write(&path, &gz)?;

  // Order: gunzip first, then checksum — so the checksum covers the PLAIN
  // bytes, proving filters run in registration order.
  let report = Pipeline::new(FileSource::new(&path), MemorySink::default())
    .filter(Box::new(GunzipFilter::new()) as Box<dyn Filter>)
    .filter(Box::new(ChecksumFilter::new(
      siwi_download::download::Algorithm::Sha256,
      expected,
    )) as Box<dyn Filter>)
    .run()
    .await?;
  assert_eq!(report.bytes_transferred, gz.len() as u64);
  Ok(())
}

#[tokio::test]
async fn test_pipeline_checksum_filter_mismatch_fails_before_finalize() -> AnyResult<()> {
  use siwi_download::pipeline::{ChecksumFilter, Filter};

  let dir = tempdir()?;
  let path = dir.path().join("plain.bin");
  std::fs::write(&path, b"some bytes")?;

  let result = Pipeline::new(FileSource::new(&path), MemorySink::default())
    .filter(Box::new(ChecksumFilter::new(
      siwi_download::download::Algorithm::Sha256,
      "0".repeat(64),
    )) as Box<dyn Filter>)
    .run()
    .await;
  let err = result.expect_err("checksum mismatch must abort the pipeline");
  assert!(err.to_string().contains("checksum mismatch"), "{err}");
  Ok(())
}

#[cfg(feature = "gzip")]
#[tokio::test]
async fn test_pipeline_tee_keeps_compressed_side_copy() -> AnyResult<()> {
  use sha2::{Digest as _, Sha256};
  use siwi_download::pipeline::{Filter, GunzipFilter, TeeFilter};
  use std::io::Write as _;

  /// Side sink capturing bytes with a hash, inspectable after the run.
  struct SideSink {
    bytes: Arc<std::sync::Mutex<Vec<u8>>>,
  }
  impl siwi_download::download::sink::StreamSink for SideSink {
    async fn write_chunk(&mut self, chunk: &[u8]) -> AnyResult<()> {
      self.bytes.lock().unwrap().extend_from_slice(chunk);
      Ok(())
    }
    async fn finalize(&mut self) -> AnyResult<()> {
      Ok(())
    }
  }

  let dir = tempdir()?;
  let plain = b"raw payload kept on the side";
  let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
  enc.write_all(plain)?;
  let gz = enc.finish()?;
  let path = dir.path().join("tee.gz");
  std::fs::write(&path, &gz)?;

  let side_bytes = Arc::new(std::sync::Mutex::new(Vec::new()));
  let side = SideSink {
    bytes: side_bytes.clone(),
  };
  // Tee BEFORE gunzip: the side sees the compressed bytes.
  let report = Pipeline::new(FileSource::new(&path), MemorySink::default())
    .filter(Box::new(TeeFilter::new(side)) as Box<dyn Filter>)
    .filter(Box::new(GunzipFilter::new()) as Box<dyn Filter>)
    .run()
    .await?;

  assert_eq!(report.bytes_transferred, gz.len() as u64);
  let captured = side_bytes.lock().unwrap().clone();
  assert_eq!(gz, captured, "tee copies the compressed bytes it receives");
  assert_eq!(
    hex::encode(Sha256::digest(&captured)),
    hex::encode(Sha256::digest(&gz))
  );
  Ok(())
}

#[tokio::test]
async fn test_stream_works_with_filtered_sink() -> AnyResult<()> {
  use sha2::{Digest as _, Sha256};
  use siwi_download::download::Download;
  use siwi_download::download::sink::StreamSink;
  use siwi_download::pipeline::{ChecksumFilter, Filter, FilteredSink};

  let body: &'static [u8] = Box::leak(b"stream through filters".to_vec().into_boxed_slice());
  let url = spawn_mock(body);

  let dir = tempdir()?;
  let download = Download::new(dir.path().to_str().unwrap());
  let plain = b"stream through filters";
  let expected = hex::encode(Sha256::digest(plain));

  // FilteredSink lets the existing stream() API use filters: the sink
  // receives plain bytes, and the checksum mismatch path would abort.
  let mut sink = FilteredSink::new(MemorySink::default()).push(Box::new(ChecksumFilter::new(
    siwi_download::download::Algorithm::Sha256,
    expected,
  )) as Box<dyn Filter>);
  let report = download
    .stream(
      &url,
      siwi_download::download::DownloadOptions::default(),
      &mut sink,
    )
    .await?;
  assert_eq!(report.file_size, Some(plain.len() as u64));

  let mut bad = FilteredSink::new(MemorySink::default()).push(Box::new(ChecksumFilter::new(
    siwi_download::download::Algorithm::Sha256,
    "0".repeat(64),
  )) as Box<dyn Filter>);
  let err = download
    .stream(
      &url,
      siwi_download::download::DownloadOptions::default(),
      &mut bad,
    )
    .await
    .expect_err("mismatch must abort stream()");
  assert!(err.to_string().contains("checksum mismatch"), "{err}");

  // The plain StreamSink trait is still implemented by the adapter.
  fn assert_stream_sink<T: StreamSink>(_: &T) {}
  assert_stream_sink(&sink);
  let _ = sink.inner();
  Ok(())
}
