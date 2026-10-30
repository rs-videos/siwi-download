//! Filters: transformations between a [`Source`](super::Source) and a
//! [`Sink`](super::Sink).
//!
//! Filters are object-safe (boxed futures) so a pipeline can hold a
//! heterogeneous `Vec<Box<dyn Filter>>`. Chunks flow through filters in
//! registration order; a filter error aborts the pipeline before the sink
//! is finalized.
//!
//! Built-ins: [`ChecksumFilter`] (verify in-flight, abort on mismatch) and,
//! with the `gzip` feature, [`GunzipFilter`] (decompress in-flight).

use crate::download::checksum::Algorithm;
use crate::download::sink::Hasher;
use crate::error::AnyResult;
use std::future::Future;
use std::pin::Pin;

/// A boxed, `Send` future — the object-safety shim for [`Filter`].
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A transformation stage between source and sink.
///
/// Both methods append their output to `out`; the pipeline pipes output
/// through the remaining filters and finally into the sink. `flush` runs
/// after the last chunk; it is the place to emit trailing state or verify
/// accumulated results.
pub trait Filter: Send {
  /// Transforms one chunk. `out` is cleared by the caller before the call.
  fn transform<'a>(
    &'a mut self,
    chunk: &'a [u8],
    out: &'a mut Vec<u8>,
  ) -> BoxFuture<'a, AnyResult<()>>;

  /// Flushes trailing state after the last chunk. `out` is cleared by the
  /// caller before the call.
  fn flush<'a>(&'a mut self, out: &'a mut Vec<u8>) -> BoxFuture<'a, AnyResult<()>> {
    let _ = out;
    Box::pin(async { Ok(()) })
  }
}

/// Verifies a checksum while bytes flow through, aborting the pipeline on
/// mismatch **before** the sink is finalized.
///
/// The filter is a pass-through: the exact bytes it receives are forwarded.
/// Place it after a decompression filter to hash the plain payload.
pub struct ChecksumFilter {
  hasher: Option<Hasher>,
  expected: String,
}

impl ChecksumFilter {
  /// Verifies against `expected_hex` (case-insensitive).
  #[must_use]
  pub fn new(algo: Algorithm, expected_hex: impl Into<String>) -> Self {
    Self {
      hasher: Some(Hasher::new(algo)),
      expected: expected_hex.into(),
    }
  }
}

impl Filter for ChecksumFilter {
  fn transform<'a>(
    &'a mut self,
    chunk: &'a [u8],
    out: &'a mut Vec<u8>,
  ) -> BoxFuture<'a, AnyResult<()>> {
    Box::pin(async move {
      if let Some(hasher) = self.hasher.as_mut() {
        hasher.update(chunk);
      }
      out.extend_from_slice(chunk);
      Ok(())
    })
  }

  fn flush<'a>(&'a mut self, _out: &'a mut Vec<u8>) -> BoxFuture<'a, AnyResult<()>> {
    Box::pin(async move {
      let Some(hasher) = self.hasher.take() else {
        return Ok(());
      };
      let actual = hasher.hex_digest();
      if actual.eq_ignore_ascii_case(self.expected.trim()) {
        Ok(())
      } else {
        Err(anyhow::anyhow!(
          "checksum mismatch: expected {}, got {actual}",
          self.expected
        ))
      }
    })
  }
}

/// Copies every byte it sees to a side sink, then passes them through.
///
/// Useful for keeping the raw (e.g. still-compressed) payload while the
/// main chain transforms it:
///
/// ```text
/// source -> TeeFilter(raw copy) -> GunzipFilter -> sink(plain)
/// ```
///
/// The side sink is finalized when the filter flushes, before the main
/// sink finalizes.
pub struct TeeFilter<S: crate::download::sink::StreamSink> {
  side: S,
}

impl<S: crate::download::sink::StreamSink> TeeFilter<S> {
  /// Creates a tee writing a copy to `side`.
  pub fn new(side: S) -> Self {
    Self { side }
  }

  /// Access to the side sink (e.g. a [`HashSink`](crate::download::sink::HashSink)
  /// to read a digest from).
  pub fn side(&self) -> &S {
    &self.side
  }
}

impl<S: crate::download::sink::StreamSink> Filter for TeeFilter<S> {
  fn transform<'a>(
    &'a mut self,
    chunk: &'a [u8],
    out: &'a mut Vec<u8>,
  ) -> BoxFuture<'a, AnyResult<()>> {
    Box::pin(async move {
      self.side.write_chunk(chunk).await?;
      out.extend_from_slice(chunk);
      Ok(())
    })
  }

  fn flush<'a>(&'a mut self, _out: &'a mut Vec<u8>) -> BoxFuture<'a, AnyResult<()>> {
    Box::pin(async move {
      self.side.finalize().await?;
      Ok(())
    })
  }
}

/// Decompresses a gzip stream in-flight (requires the `gzip` feature).
///
/// Multi-member streams are supported; each member's CRC32/ISIZE trailer is
/// verified at flush time, and truncation aborts the pipeline.
#[cfg(feature = "gzip")]
pub struct GunzipFilter {
  decoder: crate::download::sink::GunzipDecoder,
}

#[cfg(feature = "gzip")]
impl GunzipFilter {
  /// Creates a fresh gunzip filter.
  #[must_use]
  pub fn new() -> Self {
    Self {
      decoder: crate::download::sink::GunzipDecoder::new(),
    }
  }
}

#[cfg(feature = "gzip")]
impl Default for GunzipFilter {
  fn default() -> Self {
    Self::new()
  }
}

#[cfg(feature = "gzip")]
impl Filter for GunzipFilter {
  fn transform<'a>(
    &'a mut self,
    chunk: &'a [u8],
    out: &'a mut Vec<u8>,
  ) -> BoxFuture<'a, AnyResult<()>> {
    Box::pin(async move { self.decoder.feed(chunk, out) })
  }

  fn flush<'a>(&'a mut self, out: &'a mut Vec<u8>) -> BoxFuture<'a, AnyResult<()>> {
    Box::pin(async move { self.decoder.finish(out) })
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  async fn run_filter<F: Filter>(filter: &mut F, chunks: &[&[u8]]) -> AnyResult<Vec<u8>> {
    let mut collected = Vec::new();
    for chunk in chunks {
      let mut out = Vec::new();
      filter.transform(chunk, &mut out).await?;
      collected.extend_from_slice(&out);
    }
    let mut out = Vec::new();
    filter.flush(&mut out).await?;
    collected.extend_from_slice(&out);
    Ok(collected)
  }

  const HELLO_SHA256: &str = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";

  #[tokio::test]
  async fn test_checksum_filter_passes_through_and_verifies() -> AnyResult<()> {
    let mut f = ChecksumFilter::new(Algorithm::Sha256, HELLO_SHA256.to_uppercase());
    let out = run_filter(&mut f, &[b"hello ", b"world"]).await?;
    assert_eq!(b"hello world".as_slice(), out.as_slice());
    Ok(())
  }

  #[tokio::test]
  async fn test_checksum_filter_mismatch_aborts() {
    let mut f = ChecksumFilter::new(Algorithm::Sha256, "0".repeat(64));
    let err = run_filter(&mut f, &[b"hello world"])
      .await
      .expect_err("mismatch must abort");
    assert!(err.to_string().contains("checksum mismatch"), "{err}");
  }

  #[tokio::test]
  async fn test_checksum_filter_flush_is_idempotent() -> AnyResult<()> {
    let mut f = ChecksumFilter::new(Algorithm::Sha256, HELLO_SHA256);
    let mut out = Vec::new();
    f.transform(b"hello world", &mut out).await?;
    f.flush(&mut Vec::new()).await?;
    f.flush(&mut Vec::new()).await?; // second flush: no-op
    Ok(())
  }

  #[tokio::test]
  async fn test_tee_filter_copies_input_to_side_sink() -> AnyResult<()> {
    use crate::download::sink::MemorySink;
    use std::sync::{Arc, Mutex};

    struct SharedSink(Arc<Mutex<Vec<u8>>>);
    impl crate::download::sink::StreamSink for SharedSink {
      async fn write_chunk(&mut self, chunk: &[u8]) -> AnyResult<()> {
        self.0.lock().unwrap().extend_from_slice(chunk);
        Ok(())
      }
      async fn finalize(&mut self) -> AnyResult<()> {
        Ok(())
      }
    }

    let side = Arc::new(Mutex::new(Vec::new()));
    let mut tee = TeeFilter::new(SharedSink(side.clone()));
    let out = run_filter(&mut tee, &[b"abc", b"def"]).await?;

    // Main chain got the bytes; the side sink got a copy of the *input*.
    assert_eq!(b"abcdef".as_slice(), out.as_slice());
    assert_eq!(
      b"abcdef".as_slice(),
      side.lock().unwrap().as_slice(),
      "side sink copy"
    );
    let _ = MemorySink::default();
    Ok(())
  }

  #[cfg(feature = "gzip")]
  #[tokio::test]
  async fn test_gunzip_filter_decodes_across_chunks() -> AnyResult<()> {
    use std::io::Write as _;
    let plain: Vec<u8> = (0..50_000u32).map(|i| (i % 251) as u8).collect();
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(&plain)?;
    let gz = enc.finish()?;

    let mut f = GunzipFilter::new();
    let chunks: Vec<&[u8]> = gz.chunks(997).collect();
    let out = run_filter(&mut f, &chunks).await?;
    assert_eq!(plain, out);
    Ok(())
  }

  #[cfg(feature = "gzip")]
  #[tokio::test]
  async fn test_gunzip_filter_rejects_truncated_stream() -> AnyResult<()> {
    use std::io::Write as _;
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(b"payload payload payload")?;
    let gz = enc.finish()?;
    let truncated = &gz[..gz.len() - 4];

    let mut f = GunzipFilter::new();
    let mut out = Vec::new();
    f.transform(truncated, &mut out).await?;
    let err = f
      .flush(&mut Vec::new())
      .await
      .expect_err("truncation must abort");
    assert!(err.to_string().contains("incomplete gzip"), "{err}");
    Ok(())
  }
}
