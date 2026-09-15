//! File checksum computation and verification.
//!
//! Supports SHA-256, SHA-1, and MD5. File hashes are computed streaming over
//! the file contents so even multi-GB downloads won't be buffered in memory.
//!
//! # Example
//!
//! ```rust
//! use siwi_download::download::checksum::{self, Algorithm};
//!
//! let spec = "sha256:b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";
//! let (algo, expected) = checksum::parse_spec(spec).unwrap();
//! assert_eq!(algo, Algorithm::Sha256);
//! assert!(expected.len() == Algorithm::Sha256.hex_len());
//! ```

use crate::error::AnyResult;
use digest::Digest;
use md5::Md5;
use sha1::Sha1;
use sha2::Sha256;
use std::path::Path;
use tokio::{fs::File, io::AsyncReadExt};

/// Size of the streaming read buffer used when hashing files.
const HASH_READ_BUF_SIZE: usize = 64 * 1024;

/// Checksum algorithms supported by siwi-download.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Algorithm {
  /// SHA-2 with a 256-bit digest.
  Sha256,
  /// SHA-1 (160-bit). Cryptographically weak — only use when the remote
  /// site provides nothing better.
  Sha1,
  /// MD5 (128-bit). Cryptographically broken — corruption detection only.
  Md5,
}

impl Algorithm {
  /// The lowercase name used in `algo:hex` checksum specs.
  #[must_use]
  pub fn name(self) -> &'static str {
    match self {
      Algorithm::Sha256 => "sha256",
      Algorithm::Sha1 => "sha1",
      Algorithm::Md5 => "md5",
    }
  }

  /// Parses an algorithm from its name (case-insensitive). Accepts the
  /// hyphenated spelling too (`sha-256`, `sha-1`).
  #[must_use]
  pub fn from_name(name: &str) -> Option<Self> {
    match name.to_ascii_lowercase().as_str() {
      "sha256" | "sha-256" => Some(Algorithm::Sha256),
      "sha1" | "sha-1" => Some(Algorithm::Sha1),
      "md5" => Some(Algorithm::Md5),
      _ => None,
    }
  }

  /// Length in hex characters of this algorithm's digest.
  #[must_use]
  pub fn hex_len(self) -> usize {
    match self {
      Algorithm::Sha256 => 64,
      Algorithm::Sha1 => 40,
      Algorithm::Md5 => 32,
    }
  }
}

/// Parses a checksum spec of the form `algo:hex` or `algo=hex`.
///
/// The algorithm name is case-insensitive; the digest hex is returned as
/// given (comparison later is case-insensitive). The hex length is validated
/// against [`Algorithm::hex_len`].
///
/// # Errors
///
/// Returns an error if the spec is malformed, the algorithm is unknown, the
/// hex is not valid hex, or the hex length does not match the algorithm.
pub fn parse_spec(spec: &str) -> AnyResult<(Algorithm, String)> {
  let (algo_part, hex_part) = spec
    .split_once(':')
    .or_else(|| spec.split_once('='))
    .ok_or_else(|| {
      anyhow::anyhow!("invalid checksum spec `{spec}`: expected `<algo>:<hex>` or `<algo>=<hex>`")
    })?;
  let algo = Algorithm::from_name(algo_part).ok_or_else(|| {
    anyhow::anyhow!("unknown checksum algorithm `{algo_part}` (supported: sha256, sha1, md5)")
  })?;
  let hex_part = hex_part.trim();
  if hex_part.len() != algo.hex_len() {
    return Err(anyhow::anyhow!(
      "checksum hex for {algo} must be {len} characters, got {actual}",
      algo = algo.name(),
      len = algo.hex_len(),
      actual = hex_part.len()
    ));
  }
  hex::decode(hex_part)
    .map_err(|_| anyhow::anyhow!("checksum digest `{hex_part}` is not valid hex"))?;
  Ok((algo, hex_part.to_owned()))
}

/// Computes the hex-encoded digest of a file, streaming its contents.
///
/// # Errors
///
/// Returns an error if the file cannot be opened or read.
pub async fn hash_file(path: impl AsRef<Path>, algo: Algorithm) -> AnyResult<String> {
  let path = path.as_ref();
  let mut file = File::open(path)
    .await
    .map_err(|e| anyhow::anyhow!("cannot open `{}` for checksum: {e}", path.display()))?;
  match algo {
    Algorithm::Sha256 => hash_stream::<Sha256>(&mut file).await,
    Algorithm::Sha1 => hash_stream::<Sha1>(&mut file).await,
    Algorithm::Md5 => hash_stream::<Md5>(&mut file).await,
  }
}

/// Verifies a file's checksum against an expected hex digest.
///
/// Comparison is case-insensitive.
///
/// # Errors
///
/// Returns an error if the file cannot be read (e.g. it does not exist).
pub async fn verify(
  path: impl AsRef<Path>,
  algo: Algorithm,
  expected_hex: &str,
) -> AnyResult<bool> {
  let actual = hash_file(path, algo).await?;
  Ok(actual.eq_ignore_ascii_case(expected_hex.trim()))
}

/// Streams `file` through the hasher, returning the hex digest.
async fn hash_stream<D: Digest>(file: &mut File) -> AnyResult<String> {
  let mut hasher = D::new();
  let mut buf = vec![0u8; HASH_READ_BUF_SIZE];
  loop {
    let n = file.read(&mut buf).await?;
    if n == 0 {
      break;
    }
    hasher.update(&buf[..n]);
  }
  Ok(hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::io::Write;
  use tempfile::tempdir;

  const HELLO_SHA256: &str = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";
  const HELLO_SHA1: &str = "2aae6c35c94fcfb415dbe95f408b9ce91ee846ed";
  const HELLO_MD5: &str = "5eb63bbbe01eeed093cb22bb8f5acdc3";

  fn write_hello(dir: &std::path::Path) -> AnyResult<std::path::PathBuf> {
    let path = dir.join("hello.txt");
    let mut f = std::fs::File::create(&path)?;
    f.write_all(b"hello world")?;
    Ok(path)
  }

  #[tokio::test]
  async fn test_hash_file_sha256() -> AnyResult<()> {
    let dir = tempdir()?;
    let path = write_hello(dir.path())?;
    let digest = hash_file(&path, Algorithm::Sha256).await?;
    assert_eq!(HELLO_SHA256, digest);
    Ok(())
  }

  #[tokio::test]
  async fn test_hash_file_sha1() -> AnyResult<()> {
    let dir = tempdir()?;
    let path = write_hello(dir.path())?;
    let digest = hash_file(&path, Algorithm::Sha1).await?;
    assert_eq!(HELLO_SHA1, digest);
    Ok(())
  }

  #[tokio::test]
  async fn test_hash_file_md5() -> AnyResult<()> {
    let dir = tempdir()?;
    let path = write_hello(dir.path())?;
    let digest = hash_file(&path, Algorithm::Md5).await?;
    assert_eq!(HELLO_MD5, digest);
    Ok(())
  }

  #[tokio::test]
  async fn test_hash_file_large_than_buffer() -> AnyResult<()> {
    // Content larger than the 64 KiB read buffer exercises the streaming loop.
    let dir = tempdir()?;
    let path = dir.path().join("big.bin");
    let mut f = std::fs::File::create(&path)?;
    let pattern = b"0123456789".repeat(7000); // 70_000 bytes > 64 KiB
    f.write_all(&pattern)?;
    drop(f);

    // Compare streaming result against in-memory digest.
    let expected = hex::encode(Sha256::digest(&pattern));
    let actual = hash_file(&path, Algorithm::Sha256).await?;
    assert_eq!(expected, actual);
    Ok(())
  }

  #[tokio::test]
  async fn test_verify_case_insensitive() -> AnyResult<()> {
    let dir = tempdir()?;
    let path = write_hello(dir.path())?;
    let upper = HELLO_SHA256.to_uppercase();
    assert!(verify(&path, Algorithm::Sha256, &upper).await?);
    assert!(verify(&path, Algorithm::Sha256, HELLO_SHA256).await?);
    assert!(!verify(&path, Algorithm::Sha256, &"0".repeat(64)).await?);
    Ok(())
  }

  #[tokio::test]
  async fn test_verify_missing_file_errors() -> AnyResult<()> {
    let result = verify("/nonexistent/file", Algorithm::Sha256, &"0".repeat(64)).await;
    assert!(result.is_err());
    Ok(())
  }

  #[test]
  fn test_parse_spec_colon() {
    let (algo, hex_part) = parse_spec(&format!("sha256:{HELLO_SHA256}")).unwrap();
    assert_eq!(Algorithm::Sha256, algo);
    assert_eq!(HELLO_SHA256, hex_part);
  }

  #[test]
  fn test_parse_spec_equals() {
    let (algo, _) = parse_spec(&format!("md5={HELLO_MD5}")).unwrap();
    assert_eq!(Algorithm::Md5, algo);
  }

  #[test]
  fn test_parse_spec_case_insensitive_algo() {
    let (algo, _) = parse_spec(&format!("SHA1:{HELLO_SHA1}")).unwrap();
    assert_eq!(Algorithm::Sha1, algo);
  }

  #[test]
  fn test_parse_spec_rejects_unknown_algo() {
    assert!(parse_spec("crc32:01234567").is_err());
  }

  #[test]
  fn test_parse_spec_rejects_missing_separator() {
    assert!(parse_spec(HELLO_SHA256).is_err());
  }

  #[test]
  fn test_parse_spec_rejects_wrong_hex_length() {
    assert!(parse_spec("sha256:abcd").is_err());
  }

  #[test]
  fn test_parse_spec_rejects_bad_hex() {
    let bad = format!("sha256:{}", "z".repeat(64));
    assert!(parse_spec(&bad).is_err());
  }

  #[test]
  fn test_algorithm_from_name() {
    assert_eq!(Some(Algorithm::Sha256), Algorithm::from_name("sha256"));
    assert_eq!(Some(Algorithm::Sha256), Algorithm::from_name("SHA-256"));
    assert_eq!(Some(Algorithm::Sha1), Algorithm::from_name("Sha1"));
    assert_eq!(Some(Algorithm::Md5), Algorithm::from_name("MD5"));
    assert_eq!(None, Algorithm::from_name("crc32"));
  }

  #[test]
  fn test_algorithm_name_round_trip() {
    for algo in [Algorithm::Sha256, Algorithm::Sha1, Algorithm::Md5] {
      assert_eq!(Some(algo), Algorithm::from_name(algo.name()));
    }
  }
}
