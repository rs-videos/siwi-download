//! HTTP client construction helpers.
//!
//! Centralizes the [`reqwest::Client`] builder logic so that the same timeouts
//! and proxy policy are applied everywhere. Both branches (proxy vs. no-proxy)
//! share the same timeout configuration.

use crate::error::AnyResult;
use reqwest::header::HeaderMap;
use tokio::time::Duration;

/// Overall request timeout (including body) applied to every HTTP client.
const REQUEST_TIMEOUT_SECS: u64 = 60;
/// Connect-only timeout applied to every HTTP client.
const CONNECT_TIMEOUT_SECS: u64 = 10;

/// Applies the shared timeout configuration to a [`reqwest::ClientBuilder`].
///
/// All clients built in this module go through this helper so that the timeout
/// policy cannot accidentally diverge between the proxied and no-proxy paths.
fn apply_timeouts(builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
  builder
    .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
    .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
}

/// Builds an HTTP client honoring the optional `proxy`.
///
/// When `proxy` is `Some`, traffic is routed through that proxy URL; when it is
/// `None`, any proxy (including system proxies) is explicitly disabled via
/// [`reqwest::ClientBuilder::no_proxy`]. Default `headers` are merged into
/// every outgoing request.
///
/// # Errors
///
/// Returns an error if the proxy URL is invalid or the client cannot be built.
pub fn build_client(proxy: Option<&str>, headers: HeaderMap) -> AnyResult<reqwest::Client> {
  let builder = reqwest::Client::builder().default_headers(headers);
  let builder = match proxy {
    Some(proxy_url) => builder.proxy(reqwest::Proxy::all(proxy_url)?),
    None => builder.no_proxy(),
  };
  Ok(apply_timeouts(builder).build()?)
}
