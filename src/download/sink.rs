//! Streaming destination sinks for [`Download::stream`](super::Download::stream).
//!
//! A [`StreamSink`] receives the downloaded bytes chunk by chunk and is
//! finalized once the response body is exhausted. Sinks make it possible to
//! process a download without touching local disk — pipe it to stdout, hash
//! it on the fly, decompress it in flight, or fan it out to several places.
//!
//! # Example
//!
//! ```rust,no_run
//! use siwi_download::download::sink::{HashSink, MemorySink, StreamSink, TeeSink};
//! use siwi_download::download::{Download, DownloadOptions};
//!
//! # async fn example() -> siwi_download::error::AnyResult<()> {
//! let download = Download::new("/tmp");
//! let mut tee = TeeSink::new(HashSink::sha256(), MemorySink::default());
//! download
//!     .stream("https://example.com/file.zip", DownloadOptions::default(), &mut tee)
//!     .await?;
//! tee.finalize().await?;
//! println!("sha256 = {}", tee.first().hex_digest().unwrap_or_default());
//! # Ok(())
//! # }
//! ```

use crate::download::checksum::Algorithm as ChecksumAlgorithm;
use crate::error::AnyResult;
use digest::Digest;
use md5::Md5;
use sha1::Sha1;
use sha2::Sha256;
use std::future::Future;
use tokio::io::AsyncWriteExt;

#[cfg(feature = "gzip")]
use flate2::{Crc, Decompress, FlushDecompress, Status};

/// A destination for streamed download bytes.
///
/// Implementations must be [`Send`] (the download runs on a tokio task) and
/// receive chunks strictly in order. [`StreamSink::finalize`] is called
/// exactly once, after the response body is exhausted — flush pending state
/// there.
///
/// The trait uses native async-fn-in-trait; [`Download::stream`](super::Download::stream) is generic
/// over `S: StreamSink`, so no dynamic dispatch or boxing is required.
pub trait StreamSink: Send {
  /// Writes one chunk of downloaded bytes.
  ///
  /// # Errors
  ///
  /// Returning an error aborts the download.
  fn write_chunk(&mut self, chunk: &[u8]) -> impl Future<Output = AnyResult<()>> + Send;

  /// Flushes pending state after the last chunk.
  ///
  /// # Errors
  ///
  /// Returning an error fails the download.
  fn finalize(&mut self) -> impl Future<Output = AnyResult<()>> + Send {
    async { Ok(()) }
  }
}

// Bring `Future` into scope for the trait definition above.

/// Writes the download to a file on disk.
///
/// Created via [`FileSink::create`] (truncate existing content) or
/// [`FileSink::append`].
pub struct FileSink {
  file: tokio::fs::File,
}

impl FileSink {
  /// Opens (or creates) `path` for writing, truncating existing content.
  ///
  /// # Errors
  ///
  /// Returns an error if the file cannot be opened.
  pub async fn create(path: impl AsRef<std::path::Path>) -> AnyResult<Self> {
    let file = tokio::fs::File::create(path.as_ref()).await?;
    Ok(Self { file })
  }

  /// Opens `path` for appending, creating it when missing.
  ///
  /// # Errors
  ///
  /// Returns an error if the file cannot be opened.
  pub async fn append(path: impl AsRef<std::path::Path>) -> AnyResult<Self> {
    let file = tokio::fs::OpenOptions::new()
      .create(true)
      .append(true)
      .open(path.as_ref())
      .await?;
    Ok(Self { file })
  }
}

impl StreamSink for FileSink {
  async fn write_chunk(&mut self, chunk: &[u8]) -> AnyResult<()> {
    self.file.write_all(chunk).await?;
    Ok(())
  }

  async fn finalize(&mut self) -> AnyResult<()> {
    self.file.flush().await?;
    Ok(())
  }
}

/// Collects the download in memory. Intended for tests and small payloads.
#[derive(Debug, Default)]
pub struct MemorySink {
  buf: Vec<u8>,
}

impl MemorySink {
  /// Takes the collected bytes.
  #[must_use]
  pub fn into_inner(self) -> Vec<u8> {
    self.buf
  }

  /// Borrowed view of the collected bytes.
  #[must_use]
  pub fn as_slice(&self) -> &[u8] {
    &self.buf
  }
}

impl StreamSink for MemorySink {
  // The trait requires an async signature; this impl does no awaiting.
  #[allow(clippy::unused_async)]
  async fn write_chunk(&mut self, chunk: &[u8]) -> AnyResult<()> {
    self.buf.extend_from_slice(chunk);
    Ok(())
  }
}

/// Computes a checksum while the download streams — no second pass over the
/// file afterwards.
///
/// After [`StreamSink::finalize`], read the digest with
/// [`HashSink::hex_digest`].
#[derive(Debug)]
pub struct HashSink {
  inner: Hasher,
  digest_hex: Option<String>,
}

#[derive(Debug)]
enum Hasher {
  Sha256(Sha256),
  Sha1(Sha1),
  Md5(Md5),
}

impl HashSink {
  /// New sink hashing with SHA-256.
  #[must_use]
  pub fn sha256() -> Self {
    Self {
      inner: Hasher::Sha256(Sha256::new()),
      digest_hex: None,
    }
  }

  /// New sink hashing with SHA-1.
  #[must_use]
  pub fn sha1() -> Self {
    Self {
      inner: Hasher::Sha1(Sha1::new()),
      digest_hex: None,
    }
  }

  /// New sink hashing with MD5.
  #[must_use]
  pub fn md5() -> Self {
    Self {
      inner: Hasher::Md5(Md5::new()),
      digest_hex: None,
    }
  }

  /// New sink for `algo`.
  #[must_use]
  pub fn new(algo: ChecksumAlgorithm) -> Self {
    match algo {
      ChecksumAlgorithm::Sha256 => Self::sha256(),
      ChecksumAlgorithm::Sha1 => Self::sha1(),
      ChecksumAlgorithm::Md5 => Self::md5(),
    }
  }

  /// Hex digest of everything written so far.
  ///
  /// Returns `Some` only after [`StreamSink::finalize`].
  #[must_use]
  pub fn hex_digest(&self) -> Option<&str> {
    self.digest_hex.as_deref()
  }

  /// Compares the final digest against `expected_hex` (case-insensitive).
  #[must_use]
  pub fn matches(&self, expected_hex: &str) -> bool {
    self
      .digest_hex
      .as_deref()
      .is_some_and(|d| d.eq_ignore_ascii_case(expected_hex.trim()))
  }
}

impl StreamSink for HashSink {
  #[allow(clippy::unused_async)]
  async fn write_chunk(&mut self, chunk: &[u8]) -> AnyResult<()> {
    match &mut self.inner {
      Hasher::Sha256(h) => h.update(chunk),
      Hasher::Sha1(h) => h.update(chunk),
      Hasher::Md5(h) => h.update(chunk),
    }
    Ok(())
  }

  // Idempotent: `Download::stream` finalizes automatically, and callers
  // may finalize again (or inspect `hex_digest`) afterwards.
  #[allow(clippy::unused_async)]
  async fn finalize(&mut self) -> AnyResult<()> {
    if self.digest_hex.is_some() {
      return Ok(());
    }
    let hex_digest = match &mut self.inner {
      Hasher::Sha256(h) => hex::encode(std::mem::take(h).finalize()),
      Hasher::Sha1(h) => hex::encode(std::mem::take(h).finalize()),
      Hasher::Md5(h) => hex::encode(std::mem::take(h).finalize()),
    };
    self.digest_hex = Some(hex_digest);
    Ok(())
  }
}

/// Fans every chunk out to two sinks.
///
/// Wrap a [`TeeSink`] in another to reach more than two destinations.
pub struct TeeSink<A, B> {
  first: A,
  second: B,
}

impl<A, B> TeeSink<A, B>
where
  A: StreamSink,
  B: StreamSink,
{
  /// New tee writing to both `first` and `second`.
  pub fn new(first: A, second: B) -> Self {
    Self { first, second }
  }

  /// Access to the first sink (e.g. a [`HashSink`] to read a digest from).
  pub fn first(&self) -> &A {
    &self.first
  }

  /// Access to the second sink.
  pub fn second(&self) -> &B {
    &self.second
  }
}

impl<A, B> StreamSink for TeeSink<A, B>
where
  A: StreamSink + Send,
  B: StreamSink + Send,
{
  async fn write_chunk(&mut self, chunk: &[u8]) -> AnyResult<()> {
    self.first.write_chunk(chunk).await?;
    self.second.write_chunk(chunk).await?;
    Ok(())
  }

  async fn finalize(&mut self) -> AnyResult<()> {
    self.first.finalize().await?;
    self.second.finalize().await?;
    Ok(())
  }
}

/// Writes the download to standard output.
///
/// Binary-safe: use for piping (`siwi-download ... --stdout | tar -xz`).
pub struct StdoutSink;

impl StreamSink for StdoutSink {
  async fn write_chunk(&mut self, chunk: &[u8]) -> AnyResult<()> {
    let mut stdout = tokio::io::stdout();
    stdout.write_all(chunk).await?;
    stdout.flush().await?;
    Ok(())
  }
}

/// Decompresses a gzip stream on the fly and forwards the plain bytes to an
/// inner sink — download and extract in one pass, no intermediate file.
///
/// Supports multi-member streams (concatenated `.gz` files) and verifies
/// each member's CRC32/length trailer. Requires the `gzip` feature.
///
/// Implementation note: flate2's `read::MultiGzDecoder` cannot feed a
/// growing buffer (it reports any temporary input exhaustion as a hard
/// error), so this sink drives `flate2::Decompress` in raw-deflate mode and
/// parses the RFC 1952 header/trailer framing itself.
#[cfg(feature = "gzip")]
pub struct GunzipSink<S: StreamSink> {
  inner: S,
  /// Compressed bytes not yet consumed by the current phase.
  pending: Vec<u8>,
  /// Raw-deflate decompressor for the current member.
  de: Decompress,
  /// Output CRC and byte count for the current member (trailer check).
  crc: Crc,
  /// Members fully processed (header + data + verified trailer).
  members_done: u32,
  phase: GzPhase,
  scratch: Vec<u8>,
}

#[cfg(feature = "gzip")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GzPhase {
  /// Reading the RFC 1952 member header.
  Header,
  /// Inflating the raw deflate payload.
  Deflate,
  /// Reading the 8-byte CRC32/ISIZE trailer.
  Trailer,
}

#[cfg(feature = "gzip")]
const GZ_FEXTRA: u8 = 0x04;
#[cfg(feature = "gzip")]
const GZ_FNAME: u8 = 0x08;
#[cfg(feature = "gzip")]
const GZ_FCOMMENT: u8 = 0x10;
#[cfg(feature = "gzip")]
const GZ_FHCRC: u8 = 0x02;
#[cfg(feature = "gzip")]
const GZ_TRAILER_LEN: usize = 8;

#[cfg(feature = "gzip")]
impl<S: StreamSink> GunzipSink<S> {
  /// New gunzip wrapper around `inner`.
  pub fn new(inner: S) -> Self {
    Self {
      inner,
      pending: Vec::new(),
      de: Decompress::new(false),
      crc: Crc::new(),
      members_done: 0,
      phase: GzPhase::Header,
      scratch: vec![0u8; 16 * 1024],
    }
  }

  /// Access to the inner sink.
  pub fn inner(&self) -> &S {
    &self.inner
  }

  /// Advances the state machine over buffered input, forwarding decoded
  /// bytes to `inner`.
  async fn pump(&mut self) -> AnyResult<()> {
    loop {
      match self.phase {
        GzPhase::Header => {
          if self.pending.is_empty() {
            break; // waiting for input (or end of stream)
          }
          let Some(header_len) = gzip_header_len(&self.pending)? else {
            break; // header incomplete — wait for more input
          };
          self.pending.drain(..header_len);
          self.de = Decompress::new(false);
          self.crc = Crc::new();
          self.phase = GzPhase::Deflate;
        }
        GzPhase::Deflate => {
          let before_in = self.de.total_in();
          let before_out = self.de.total_out();
          let (status, produced) = {
            let input = self.pending.as_slice();
            let scratch = &mut self.scratch;
            // Disjoint borrows via field split; `decompress` reads `input`
            // and writes `scratch`.
            let status = self.de.decompress(input, scratch, FlushDecompress::None)?;
            let produced = usize::try_from(self.de.total_out() - before_out).unwrap_or(usize::MAX);
            (status, produced)
          };
          let consumed = usize::try_from(self.de.total_in() - before_in).unwrap_or(usize::MAX);
          self.pending.drain(..consumed);
          if produced > 0 {
            self.crc.update(&self.scratch[..produced]);
            self.inner.write_chunk(&self.scratch[..produced]).await?;
          }
          match status {
            Status::StreamEnd => self.phase = GzPhase::Trailer,
            Status::Ok | Status::BufError => {
              if produced == 0 && consumed == 0 {
                break; // need more input
              }
            }
          }
        }
        GzPhase::Trailer => {
          if self.pending.len() < GZ_TRAILER_LEN {
            break; // trailer incomplete — wait for more input
          }
          let trailer: [u8; GZ_TRAILER_LEN] = self.pending[..GZ_TRAILER_LEN]
            .try_into()
            .expect("fixed len");
          self.pending.drain(..GZ_TRAILER_LEN);
          let crc_expected = u32::from_le_bytes(trailer[..4].try_into().expect("4 bytes"));
          let isize_expected = u32::from_le_bytes(trailer[4..].try_into().expect("4 bytes"));
          if self.crc.sum() != crc_expected {
            return Err(anyhow::anyhow!(
              "gzip CRC mismatch: got {:08x}, trailer says {:08x}",
              self.crc.sum(),
              crc_expected
            ));
          }
          if self.crc.amount() != isize_expected {
            return Err(anyhow::anyhow!(
              "gzip length mismatch: got {}, trailer says {}",
              self.crc.amount(),
              isize_expected
            ));
          }
          self.members_done += 1;
          // Multi-member: another member may follow immediately.
          self.phase = GzPhase::Header;
        }
      }
    }
    Ok(())
  }
}

/// Returns the byte length of the gzip member header at the start of `buf`,
/// or `None` when more bytes are needed to decide.
#[cfg(feature = "gzip")]
fn gzip_header_len(buf: &[u8]) -> AnyResult<Option<usize>> {
  const GZ_MAGIC: [u8; 2] = [0x1f, 0x8b];
  const GZ_DEFLATE_METHOD: u8 = 8;
  if buf.len() < 10 {
    return Ok(None);
  }
  if buf[..2] != GZ_MAGIC {
    return Err(anyhow::anyhow!("not a gzip stream (bad magic bytes)"));
  }
  if buf[2] != GZ_DEFLATE_METHOD {
    return Err(anyhow::anyhow!(
      "unsupported gzip compression method {}",
      buf[2]
    ));
  }
  let flg = buf[3];
  let mut p = 10usize;
  if flg & GZ_FEXTRA != 0 {
    if buf.len() < p + 2 {
      return Ok(None);
    }
    let xlen = u16::from_le_bytes([buf[p], buf[p + 1]]) as usize;
    p += 2 + xlen;
    if buf.len() < p {
      return Ok(None);
    }
  }
  if flg & GZ_FNAME != 0 {
    let Some(nul) = buf[p..].iter().position(|&b| b == 0) else {
      return Ok(None);
    };
    p += nul + 1;
  }
  if flg & GZ_FCOMMENT != 0 {
    let Some(nul) = buf[p..].iter().position(|&b| b == 0) else {
      return Ok(None);
    };
    p += nul + 1;
  }
  if flg & GZ_FHCRC != 0 {
    p += 2;
  }
  if buf.len() < p {
    return Ok(None);
  }
  Ok(Some(p))
}

#[cfg(feature = "gzip")]
impl<S: StreamSink> StreamSink for GunzipSink<S> {
  async fn write_chunk(&mut self, chunk: &[u8]) -> AnyResult<()> {
    self.pending.extend_from_slice(chunk);
    self.pump().await
  }

  async fn finalize(&mut self) -> AnyResult<()> {
    self.pump().await?;
    // Exactly one of these must hold for a well-formed stream:
    // - the last member's trailer was verified (phase wrapped to Header and
    //   no bytes followed), or
    // - nothing is pending mid-structure.
    if self.members_done == 0 || self.phase != GzPhase::Header || !self.pending.is_empty() {
      return Err(anyhow::anyhow!(
        "incomplete gzip stream ({} member(s) verified, phase {:?}, {} trailing bytes)",
        self.members_done,
        self.phase,
        self.pending.len()
      ));
    }
    self.inner.finalize().await?;
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  async fn feed(sink: &mut impl StreamSink, chunks: &[&[u8]]) -> AnyResult<()> {
    for c in chunks {
      sink.write_chunk(c).await?;
    }
    sink.finalize().await?;
    Ok(())
  }

  #[tokio::test]
  async fn test_memory_sink_collects_chunks() -> AnyResult<()> {
    let mut sink = MemorySink::default();
    feed(&mut sink, &[b"hello", b" ", b"world"]).await?;
    assert_eq!(b"hello world".as_slice(), sink.as_slice());
    assert_eq!(b"hello world".to_vec(), sink.into_inner());
    Ok(())
  }

  #[tokio::test]
  async fn test_file_sink_create_and_append() -> AnyResult<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("out.bin");

    {
      let mut sink = FileSink::create(&path).await?;
      feed(&mut sink, &[b"first"]).await?;
    }
    {
      let mut sink = FileSink::append(&path).await?;
      feed(&mut sink, &[b"+second"]).await?;
    }
    assert_eq!(b"first+second".to_vec(), std::fs::read(&path)?);
    Ok(())
  }

  #[tokio::test]
  async fn test_hash_sink_known_vectors() -> AnyResult<()> {
    for (algo, expected) in [
      (
        ChecksumAlgorithm::Sha256,
        "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9",
      ),
      (
        ChecksumAlgorithm::Sha1,
        "2aae6c35c94fcfb415dbe95f408b9ce91ee846ed",
      ),
      (ChecksumAlgorithm::Md5, "5eb63bbbe01eeed093cb22bb8f5acdc3"),
    ] {
      let mut sink = HashSink::new(algo);
      feed(&mut sink, &[b"hello ", b"world"]).await?;
      assert_eq!(Some(expected), sink.hex_digest(), "algo {algo:?}");
      assert!(sink.matches(expected));
      assert!(sink.matches(&expected.to_uppercase()));
      assert!(!sink.matches(&"0".repeat(expected.len())));
    }
    Ok(())
  }

  #[tokio::test]
  async fn test_hash_sink_digest_none_before_finalize() {
    let mut sink = HashSink::sha256();
    sink.write_chunk(b"x").await.unwrap();
    assert!(sink.hex_digest().is_none());
  }

  #[tokio::test]
  async fn test_tee_sink_fans_out() -> AnyResult<()> {
    let mem = MemorySink::default();
    let hash = HashSink::sha256();
    let mut tee = TeeSink::new(mem, hash);
    feed(&mut tee, &[b"abc"]).await?;
    assert_eq!(b"abc".as_slice(), tee.first().as_slice());
    assert_eq!(
      Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"),
      tee.second().hex_digest()
    );
    Ok(())
  }

  #[tokio::test]
  async fn test_stdout_sink_smoke() -> AnyResult<()> {
    // Not asserting on real stdout content; just proving write paths work.
    let mut sink = StdoutSink;
    sink.write_chunk(b"").await?;
    sink.finalize().await?;
    Ok(())
  }

  #[cfg(feature = "gzip")]
  #[tokio::test]
  async fn test_gunzip_sink_decodes_across_chunks() -> AnyResult<()> {
    use std::io::Write as _;

    // Compress a payload larger than one chunk, then feed it in odd-sized
    // slices to exercise the streaming decoder across chunk boundaries.
    let plain: Vec<u8> = (0..70_000u32).map(|i| (i % 251) as u8).collect();
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&plain)?;
    let gz = encoder.finish()?;

    let mem = MemorySink::default();
    let mut sink = GunzipSink::new(mem);
    for chunk in gz.chunks(997) {
      sink.write_chunk(chunk).await?;
    }
    sink.finalize().await?;
    assert_eq!(plain, sink.inner().as_slice());
    Ok(())
  }
}
