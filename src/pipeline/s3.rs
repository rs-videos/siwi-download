//! Minimal S3 support over plain HTTP with `SigV4` header auth — **no AWS
//! SDK**.
//!
//! ADR-007 originally planned `aws-sdk-s3` behind the `s3` feature. In
//! practice the SDK pulls a large dependency tree and constrains the MSRV;
//! siwi-download needs only three operations (`HeadObject`, `GetObject`,
//! `PutObject`), so the `s3` feature instead adds a single tiny dependency
//! (`hmac`) and signs requests itself. The signer is verified against two
//! official AWS S3 `SigV4` test vectors in this module's tests.
//!
//! Gated behind **both** `unstable` (pipeline) and `s3`.
//!
//! # Example
//!
//! ```rust,no_run
//! use siwi_download::pipeline::{Pipeline, S3Config, S3Source};
//! use siwi_download::download::sink::MemorySink;
//!
//! # async fn example() -> siwi_download::error::AnyResult<()> {
//! let config = S3Config {
//!     endpoint: "https://s3.us-east-1.amazonaws.com".into(),
//!     region: "us-east-1".into(),
//!     bucket: "my-bucket".into(),
//!     access_key: std::env::var("AWS_ACCESS_KEY_ID")?,
//!     secret_key: std::env::var("AWS_SECRET_ACCESS_KEY")?,
//!     session_token: None,
//!     path_style: false,
//! };
//! let source = S3Source::new(config, "data/file.bin")?;
//! let report = Pipeline::new(source, MemorySink::default()).run().await?;
//! println!("{} bytes", report.bytes_transferred);
//! # Ok(())
//! # }
//! ```

use super::source::{Source, SourceMetadata};
use crate::download::client;
use crate::error::AnyResult;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::{Method, Url};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;

type HmacSha256 = Hmac<Sha256>;

/// SHA-256 of the empty payload, used for bodyless requests.
const EMPTY_PAYLOAD_SHA256: &str =
  "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
/// Default in-memory spool ceiling for [`S3Sink`].
const DEFAULT_MAX_BUFFER: usize = 64 * 1024 * 1024;

/// Connection + credential settings for S3.
#[derive(Clone)]
pub struct S3Config {
  /// Service endpoint, e.g. `https://s3.us-east-1.amazonaws.com` or a
  /// minio URL such as `http://127.0.0.1:9000`.
  pub endpoint: String,
  /// Signing region, e.g. `us-east-1`.
  pub region: String,
  /// Bucket name.
  pub bucket: String,
  /// Access key id.
  pub access_key: String,
  /// Secret access key.
  pub secret_key: String,
  /// Optional temporary-session token (`x-amz-security-token`).
  pub session_token: Option<String>,
  /// Use path-style addressing (`endpoint/bucket/key`). Required by minio;
  /// AWS still supports it for most endpoints. Virtual-host style
  /// (`bucket.endpoint/key`) when `false`.
  pub path_style: bool,
}

impl std::fmt::Debug for S3Config {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("S3Config")
      .field("endpoint", &self.endpoint)
      .field("region", &self.region)
      .field("bucket", &self.bucket)
      .field("access_key", &self.access_key)
      .field("secret_key", &"<redacted>")
      .field(
        "session_token",
        &self.session_token.as_ref().map(|_| "<redacted>"),
      )
      .field("path_style", &self.path_style)
      .finish()
  }
}

/// AWS `SigV4` header-based signer for the `s3` service.
pub(crate) struct SigV4Signer<'a> {
  config: &'a S3Config,
}

impl<'a> SigV4Signer<'a> {
  pub(crate) fn new(config: &'a S3Config) -> Self {
    Self { config }
  }

  /// Signs `headers` in place for `method` + `url`, inserting
  /// `x-amz-date`, `x-amz-content-sha256` (if absent), and `Authorization`.
  ///
  /// `payload_hash` must be the lowercase hex SHA-256 of the body (use
  /// [`EMPTY_PAYLOAD_SHA256`] for bodyless verbs).
  ///
  /// # Errors
  ///
  /// Returns an error for malformed URLs/header values.
  pub(crate) fn sign(
    &self,
    method: &Method,
    url: &Url,
    headers: &mut HeaderMap,
    payload_hash: &str,
    now: DateTime<Utc>,
  ) -> AnyResult<()> {
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let date_stamp = now.format("%Y%m%d").to_string();

    headers.insert(
      HeaderName::from_static("x-amz-date"),
      HeaderValue::from_str(&amz_date)?,
    );
    headers.insert(
      HeaderName::from_static("x-amz-content-sha256"),
      HeaderValue::from_str(payload_hash)?,
    );
    if let Some(token) = self.config.session_token.as_ref() {
      headers.insert(
        HeaderName::from_static("x-amz-security-token"),
        HeaderValue::from_str(token)?,
      );
    }

    // Host header (include the port when non-default).
    let host = match url.port() {
      Some(port) => format!("{}:{port}", url.host_str().unwrap_or_default()),
      None => url.host_str().unwrap_or_default().to_string(),
    };
    headers.insert(reqwest::header::HOST, HeaderValue::from_str(&host)?);

    // Sign exactly: host, range/date when present, and every x-amz-*
    // header. AWS requires all x-amz-* headers to be signed.
    let mut signed: Vec<(String, String)> = Vec::new();
    for (name, value) in headers.iter() {
      let name_str = name.as_str().to_ascii_lowercase();
      let is_signed = name_str == "host"
        || name_str == "range"
        || name_str == "date"
        || name_str.starts_with("x-amz-");
      if is_signed {
        let value_str = value
          .to_str()
          .map_err(|e| anyhow::anyhow!("non-ascii header {name_str}: {e}"))?
          .trim()
          .to_string();
        signed.push((name_str, value_str));
      }
    }
    signed.sort();
    let mut canonical_headers = String::new();
    for (key, value) in &signed {
      let _ = writeln!(canonical_headers, "{key}:{value}");
    }
    let signed_headers: String = signed
      .iter()
      .map(|(k, _)| k.as_str())
      .collect::<Vec<_>>()
      .join(";");

    // `Url::path()` is already percent-encoded (RFC 3986); re-encoding it
    // would turn `%24` into `%2524` and break the signature.
    let canonical_request = format!(
      "{}\n{}\n\n{}\n{}\n{}",
      method.as_str(),
      url.path(),
      canonical_headers,
      signed_headers,
      payload_hash
    );

    let scope = format!(
      "{date_stamp}/{region}/s3/aws4_request",
      region = self.config.region
    );
    let string_to_sign = format!(
      "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
      hex::encode(Sha256::digest(canonical_request.as_bytes()))
    );

    let signing_key = signing_key(&self.config.secret_key, &date_stamp, &self.config.region);
    let signature = hex::encode(hmac_sha256(&signing_key, string_to_sign.as_bytes()));

    let authorization = format!(
      "AWS4-HMAC-SHA256 Credential={ak}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
      ak = self.config.access_key
    );
    headers.insert(
      reqwest::header::AUTHORIZATION,
      HeaderValue::from_str(&authorization)?,
    );
    Ok(())
  }
}

/// Derives the `SigV4` signing key (`AWS4` secret → date → region → service).
fn signing_key(secret: &str, date_stamp: &str, region: &str) -> Vec<u8> {
  let k_date = hmac_sha256(format!("AWS4{secret}").as_bytes(), date_stamp.as_bytes());
  let k_region = hmac_sha256(&k_date, region.as_bytes());
  let k_service = hmac_sha256(&k_region, b"s3");
  hmac_sha256(&k_service, b"aws4_request")
}

/// One-shot `HMAC-SHA256`.
fn hmac_sha256(key: &[u8], msg: &[u8]) -> Vec<u8> {
  let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
  mac.update(msg);
  mac.finalize().into_bytes().to_vec()
}

/// AWS canonical URI: percent-encode each path segment, preserving `/`.
fn canonical_uri(path: &str) -> String {
  let mut out = String::with_capacity(path.len());
  for segment in path.split('/') {
    if !out.is_empty() || path.starts_with('/') {
      out.push('/');
    }
    for byte in segment.bytes() {
      match byte {
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
          out.push(byte as char);
        }
        _ => {
          let _ = write!(out, "%{byte:02X}");
        }
      }
    }
  }
  if out.starts_with("//") {
    // path was "/" — keep a single leading slash
    out = out[1..].to_string();
  }
  out
}

impl S3Config {
  /// Builds the object URL for `key` according to the addressing style.
  fn object_url(&self, key: &str) -> AnyResult<Url> {
    let encoded_key = canonical_uri(key.trim_start_matches('/'));
    let url = if self.path_style {
      let base = self.endpoint.trim_end_matches('/');
      Url::parse(&format!("{base}/{}/{}", self.bucket, encoded_key))?
    } else {
      let parsed = Url::parse(self.endpoint.trim_end_matches('/'))?;
      let host = parsed
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("S3 endpoint has no host: {}", self.endpoint))?;
      let virtual_host = format!("{}.{host}", self.bucket);
      let port_part = parsed.port().map(|p| format!(":{p}")).unwrap_or_default();
      Url::parse(&format!(
        "{}://{virtual_host}{port_part}/{encoded_key}",
        parsed.scheme()
      ))?
    };
    Ok(url)
  }
}

/// Builds the HTTP client used by S3 sources/sinks.
fn build_http_client() -> AnyResult<reqwest::Client> {
  client::build_client(None, HeaderMap::new())
}

/// Reads an S3 object.
pub struct S3Source {
  config: S3Config,
  key: String,
  client: reqwest::Client,
  resp: Option<reqwest::Response>,
}

impl S3Source {
  /// Creates a source for `key` in the configured bucket.
  ///
  /// # Errors
  ///
  /// Returns an error when the endpoint URL is invalid or the HTTP client
  /// cannot be built.
  pub fn new(config: S3Config, key: impl Into<String>) -> AnyResult<Self> {
    let client = build_http_client()?;
    Ok(Self {
      config,
      key: key.into(),
      client,
      resp: None,
    })
  }

  /// The object key this source reads.
  #[must_use]
  pub fn key(&self) -> &str {
    &self.key
  }

  fn signed_headers(
    &self,
    method: &Method,
    payload_hash: &str,
    ranged: bool,
    range_from: u64,
  ) -> AnyResult<(Url, HeaderMap)> {
    let url = self.config.object_url(&self.key)?;
    let mut headers = HeaderMap::new();
    if ranged && range_from > 0 {
      headers.insert(
        reqwest::header::RANGE,
        HeaderValue::from_str(&format!("bytes={range_from}-"))?,
      );
    }
    SigV4Signer::new(&self.config).sign(method, &url, &mut headers, payload_hash, Utc::now())?;
    Ok((url, headers))
  }
}

impl Source for S3Source {
  async fn metadata(&self) -> AnyResult<SourceMetadata> {
    let (url, headers) = self.signed_headers(&Method::HEAD, EMPTY_PAYLOAD_SHA256, true, 0)?;
    let resp = self
      .client
      .head(url.clone())
      .headers(headers)
      .send()
      .await?;
    let status = resp.status().as_u16();
    if !resp.status().is_success() {
      return Err(anyhow::anyhow!(
        "S3 HeadObject failed for {}/{}: HTTP {status}",
        self.config.bucket,
        self.key
      ));
    }
    let content_length = resp
      .headers()
      .get(reqwest::header::CONTENT_LENGTH)
      .and_then(|hv| hv.to_str().ok())
      .and_then(|l| l.parse::<u64>().ok());
    let etag = resp
      .headers()
      .get(reqwest::header::ETAG)
      .and_then(|hv| hv.to_str().ok())
      .map(ToOwned::to_owned);
    let last_modified = resp
      .headers()
      .get(reqwest::header::LAST_MODIFIED)
      .and_then(|hv| hv.to_str().ok())
      .and_then(|raw| DateTime::parse_from_rfc2822(raw).ok())
      .map(|dt| dt.with_timezone(&Utc));
    Ok(SourceMetadata {
      location: format!("s3://{}/{}", self.config.bucket, self.key),
      status,
      content_length,
      etag,
      last_modified,
      accepts_range: true,
    })
  }

  async fn open(&mut self, range_from: u64) -> AnyResult<()> {
    let (url, headers) =
      self.signed_headers(&Method::GET, EMPTY_PAYLOAD_SHA256, true, range_from)?;
    let resp = self.client.get(url).headers(headers).send().await?;
    if !resp.status().is_success() {
      return Err(anyhow::anyhow!(
        "S3 GetObject failed for {}/{}: HTTP {}",
        self.config.bucket,
        self.key,
        resp.status()
      ));
    }
    self.resp = Some(resp);
    Ok(())
  }

  async fn next_chunk(&mut self) -> AnyResult<Option<Bytes>> {
    match self.resp.as_mut() {
      Some(resp) => Ok(resp.chunk().await?),
      None => Err(anyhow::anyhow!("source not opened; call open() first")),
    }
  }
}

/// Uploads a pipeline into an S3 object with a single `PutObject`.
///
/// Bytes are spooled in memory up to `max_buffer` (default 64 MiB) so the
/// request carries an exact `Content-Length` and payload hash — S3 requires
/// both for a plain `PutObject`, and multipart upload is out of scope for the
/// first S3 slice. An upload exceeding the buffer fails rather than spilling
/// to disk.
pub struct S3Sink {
  config: S3Config,
  key: String,
  client: reqwest::Client,
  buffer: Vec<u8>,
  max_buffer: usize,
  finalized: bool,
}

impl S3Sink {
  /// Creates a sink uploading to `key`.
  ///
  /// # Errors
  ///
  /// Returns an error when the endpoint URL is invalid or the HTTP client
  /// cannot be built.
  pub fn new(config: S3Config, key: impl Into<String>) -> AnyResult<Self> {
    let client = build_http_client()?;
    Ok(Self {
      config,
      key: key.into(),
      client,
      buffer: Vec::new(),
      max_buffer: DEFAULT_MAX_BUFFER,
      finalized: false,
    })
  }

  /// Overrides the in-memory spool ceiling.
  #[must_use]
  pub fn max_buffer(mut self, bytes: usize) -> Self {
    self.max_buffer = bytes;
    self
  }

  /// The object key this sink uploads to.
  #[must_use]
  pub fn key(&self) -> &str {
    &self.key
  }
}

impl crate::download::sink::StreamSink for S3Sink {
  async fn write_chunk(&mut self, chunk: &[u8]) -> AnyResult<()> {
    if self.buffer.len() + chunk.len() > self.max_buffer {
      return Err(anyhow::anyhow!(
        "S3 sink buffer exceeded ({} bytes); raise max_buffer or use a presigned upload for large objects",
        self.max_buffer
      ));
    }
    self.buffer.extend_from_slice(chunk);
    Ok(())
  }

  async fn finalize(&mut self) -> AnyResult<()> {
    if self.finalized {
      return Ok(());
    }
    let url = self.config.object_url(&self.key)?;
    let payload_hash = hex::encode(Sha256::digest(&self.buffer));
    let mut headers = HeaderMap::new();
    // Content-Length is derived by reqwest from the Vec body; SigV4 does not
    // require signing it.
    SigV4Signer::new(&self.config).sign(
      &Method::PUT,
      &url,
      &mut headers,
      &payload_hash,
      Utc::now(),
    )?;

    let resp = self
      .client
      .put(url)
      .headers(headers)
      .body(std::mem::take(&mut self.buffer))
      .send()
      .await?;
    if !resp.status().is_success() {
      return Err(anyhow::anyhow!(
        "S3 PutObject failed for {}/{}: HTTP {}",
        self.config.bucket,
        self.key,
        resp.status()
      ));
    }
    self.finalized = true;
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use chrono::TimeZone;

  fn example_config() -> S3Config {
    S3Config {
      endpoint: "https://examplebucket.s3.amazonaws.com".to_string(),
      region: "us-east-1".to_string(),
      bucket: "examplebucket".to_string(),
      access_key: "AKIAIOSFODNN7EXAMPLE".to_string(),
      secret_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string(),
      session_token: None,
      path_style: false,
    }
  }

  fn fixed_now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2013, 5, 24, 0, 0, 0).single().unwrap()
  }

  /// Official AWS S3 "Example: GET Object" vector (Range header included).
  #[test]
  fn test_sigv4_matches_aws_get_object_vector() -> AnyResult<()> {
    let config = example_config();
    let url = Url::parse("https://examplebucket.s3.amazonaws.com/test.txt")?;
    let mut headers = HeaderMap::new();
    headers.insert(reqwest::header::RANGE, HeaderValue::from_str("bytes=0-9")?);

    SigV4Signer::new(&config).sign(
      &Method::GET,
      &url,
      &mut headers,
      EMPTY_PAYLOAD_SHA256,
      fixed_now(),
    )?;

    let auth = headers
      .get(reqwest::header::AUTHORIZATION)
      .expect("authorization set")
      .to_str()?;
    let expected = "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, SignedHeaders=host;range;x-amz-content-sha256;x-amz-date, Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41";
    assert_eq!(expected, auth);
    Ok(())
  }

  /// Official AWS S3 "Example: PUT Object" vector (Date + storage class).
  #[test]
  fn test_sigv4_matches_aws_put_object_vector() -> AnyResult<()> {
    let config = example_config();
    let url = Url::parse("https://examplebucket.s3.amazonaws.com/test%24file.text")?;
    let mut headers = HeaderMap::new();
    headers.insert(
      reqwest::header::DATE,
      HeaderValue::from_str("Fri, 24 May 2013 00:00:00 GMT")?,
    );
    headers.insert(
      HeaderName::from_static("x-amz-storage-class"),
      HeaderValue::from_str("REDUCED_REDUNDANCY")?,
    );
    let payload_hash = hex::encode(Sha256::digest(b"Welcome to Amazon S3."));

    SigV4Signer::new(&config).sign(&Method::PUT, &url, &mut headers, &payload_hash, fixed_now())?;

    let auth = headers
      .get(reqwest::header::AUTHORIZATION)
      .expect("authorization set")
      .to_str()?;
    let expected = "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, SignedHeaders=date;host;x-amz-content-sha256;x-amz-date;x-amz-storage-class, Signature=98ad721746da40c64f1a55b78f14c238d841ea1380cd77a1b5971af0ece108bd";
    assert_eq!(expected, auth);
    Ok(())
  }

  #[test]
  fn test_sigv4_signs_session_token() -> AnyResult<()> {
    let mut config = example_config();
    config.session_token = Some("token-123".to_string());
    let url = Url::parse("https://examplebucket.s3.amazonaws.com/k")?;
    let mut headers = HeaderMap::new();
    SigV4Signer::new(&config).sign(
      &Method::GET,
      &url,
      &mut headers,
      EMPTY_PAYLOAD_SHA256,
      fixed_now(),
    )?;
    let auth = headers
      .get(reqwest::header::AUTHORIZATION)
      .expect("authorization set")
      .to_str()?;
    assert!(
      auth.contains("x-amz-security-token"),
      "session token must be signed: {auth}"
    );
    assert!(
      headers.get("x-amz-security-token").is_some(),
      "session token header present"
    );
    Ok(())
  }

  #[test]
  fn test_canonical_uri_encoding() {
    assert_eq!("/test%24file.text", canonical_uri("/test$file.text"));
    assert_eq!("/a/b/c.txt", canonical_uri("/a/b/c.txt"));
    assert_eq!("/a%20b", canonical_uri("/a b"));
    assert_eq!("/", canonical_uri("/"));
    assert_eq!("/%E4%BD%A0", canonical_uri("/你"));
  }

  #[test]
  fn test_object_url_addressing_styles() -> AnyResult<()> {
    let mut config = example_config();
    config.endpoint = "https://s3.amazonaws.com".to_string();
    config.path_style = false;
    let url = config.object_url("a/b c.txt")?;
    assert_eq!(
      "https://examplebucket.s3.amazonaws.com/a/b%20c.txt",
      url.as_str()
    );

    config.path_style = true;
    config.endpoint = "http://127.0.0.1:9000".to_string();
    let url = config.object_url(" a/b.txt ".trim_start_matches('/'));
    // trailing spaces in keys are legal; assert only the addressing shape
    let url = url?;
    assert!(
      url
        .as_str()
        .starts_with("http://127.0.0.1:9000/examplebucket/")
    );
    Ok(())
  }

  #[test]
  fn test_signing_key_is_deterministic() {
    let a = signing_key("secret", "20130524", "us-east-1");
    let b = signing_key("secret", "20130524", "us-east-1");
    assert_eq!(a, b);
    let c = signing_key("secret", "20130524", "eu-west-1");
    assert_ne!(a, c, "region changes the signing key");
  }
}
