//! CLI configuration layering for the `siwi-download` binary.
//!
//! This module is compiled into the binary target only; the library API is
//! unaffected. Options are resolved with the precedence:
//!
//! ```text
//! CLI flag > environment variable > config file > built-in default
//! ```
//!
//! # Config file
//!
//! TOML, discovered at the platform config directory unless `--config`
//! names one explicitly:
//!
//! - Linux:   `$XDG_CONFIG_HOME/siwi-download/config.toml` or `~/.config/siwi-download/config.toml`
//! - macOS:   `~/Library/Application Support/siwi-download/config.toml`
//! - Windows: `%APPDATA%\siwi-download\config.toml`
//!
//! ```toml
//! [default]
//! output = "./downloads"
//! progress = true
//! max_speed = "20M"     # 1024-based K/M/G suffixes
//!
//! [proxy]
//! url = "http://127.0.0.1:7890"
//! ```
//!
//! # Environment variables
//!
//! - `SIWI_DOWNLOAD_OUTPUT`
//! - `SIWI_DOWNLOAD_PROGRESS` (`true`/`1`/`yes` or `false`/`0`/`no`)
//! - `SIWI_DOWNLOAD_MAX_SPEED` (same syntax as `--max-speed`)
//! - `SIWI_DOWNLOAD_PROXY`

use anyhow::Context;
use serde::Deserialize;
use siwi_download::error::AnyResult;
use std::path::PathBuf;

/// The on-disk config file schema (TOML).
///
/// Unknown keys are rejected so typos (e.g. `max_sped`) fail loudly instead
/// of silently doing nothing.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileConfig {
  /// `[default]` section: settings applied to every download.
  #[serde(default)]
  pub default: Defaults,
  /// `[proxy]` section.
  #[serde(default)]
  pub proxy: Proxy,
}

/// `[default]` section of the config file.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Defaults {
  /// Default output directory.
  pub output: Option<String>,
  /// Show the progress bar by default.
  pub progress: Option<bool>,
  /// Cap the average speed, e.g. `"20M"`.
  pub max_speed: Option<String>,
}

/// `[proxy]` section of the config file.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proxy {
  /// Proxy URL, e.g. `"http://127.0.0.1:7890"`.
  pub url: Option<String>,
}

/// Settings after layering env vars over the config file (CLI flags are
/// applied on top by the caller).
#[derive(Debug, Default)]
pub struct Resolved {
  /// Output directory, if any layer provided one.
  pub output: Option<String>,
  /// Progress bar preference, if any layer provided one.
  pub progress: Option<bool>,
  /// Speed cap in bytes per second, if any layer provided one.
  pub max_speed_bytes: Option<u64>,
  /// Proxy URL, if any layer provided one.
  pub proxy: Option<String>,
}

/// Returns the default config file path for this platform, or `None` if the
/// platform has no config directory.
#[must_use]
pub fn default_path() -> Option<PathBuf> {
  dirs::config_dir().map(|d| d.join("siwi-download").join("config.toml"))
}

/// Loads the config file.
///
/// With `explicit` (`--config`), a missing or invalid file is an error.
/// Without it, a missing default file yields `Ok(None)`; an invalid one is
/// an error so typos never silently change behavior.
///
/// # Errors
///
/// Returns an error when the file cannot be read or parsed.
pub fn load(explicit: Option<&str>) -> AnyResult<Option<FileConfig>> {
  let path = match explicit {
    Some(p) => PathBuf::from(p),
    None => match default_path() {
      Some(p) => p,
      None => return Ok(None),
    },
  };
  if !path.exists() {
    if explicit.is_some() {
      return Err(anyhow::anyhow!("config file not found: {}", path.display()));
    }
    return Ok(None);
  }
  let raw = std::fs::read_to_string(&path)
    .with_context(|| format!("cannot read config file {}", path.display()))?;
  let cfg: FileConfig =
    toml::from_str(&raw).with_context(|| format!("cannot parse config file {}", path.display()))?;
  Ok(Some(cfg))
}

/// Layers env vars over the config file. `env` is injected for testability.
///
/// # Errors
///
/// Returns an error for unparseable `SIWI_DOWNLOAD_PROGRESS` or
/// `SIWI_DOWNLOAD_MAX_SPEED` values.
pub fn resolve(
  file: Option<&FileConfig>,
  env: &dyn Fn(&str) -> Option<String>,
) -> AnyResult<Resolved> {
  let file = file.cloned().unwrap_or_default();
  // Empty env values count as unset so `SIWI_DOWNLOAD_PROXY= siwi-download ...`
  // does not wipe the config file's proxy.
  let get = |k: &str| {
    env(k)
      .map(|v| v.trim().to_owned())
      .filter(|v| !v.is_empty())
  };

  let output = get("SIWI_DOWNLOAD_OUTPUT").or(file.default.output);

  let progress = match get("SIWI_DOWNLOAD_PROGRESS") {
    Some(v) => Some(parse_bool(&v).ok_or_else(|| {
      anyhow::anyhow!("invalid SIWI_DOWNLOAD_PROGRESS `{v}`: expected true/false/1/0/yes/no")
    })?),
    None => file.default.progress,
  };

  let max_speed_spec = get("SIWI_DOWNLOAD_MAX_SPEED").or(file.default.max_speed);
  let max_speed_bytes = match max_speed_spec.as_deref() {
    Some(spec) => Some(
      parse_speed_spec(spec)
        .map_err(|e| anyhow::anyhow!("{e} (from config file or SIWI_DOWNLOAD_MAX_SPEED)"))?,
    ),
    None => None,
  };

  let proxy = get("SIWI_DOWNLOAD_PROXY").or(file.proxy.url);

  Ok(Resolved {
    output,
    progress,
    max_speed_bytes,
    proxy,
  })
}

/// Parses `true`/`1`/`yes`/`false`/`0`/`no` (case-insensitive).
fn parse_bool(v: &str) -> Option<bool> {
  match v.to_ascii_lowercase().as_str() {
    "true" | "1" | "yes" => Some(true),
    "false" | "0" | "no" => Some(false),
    _ => None,
  }
}

/// Parses a speed spec like `10M` into bytes per second (1024-based).
///
/// Accepts plain byte counts and the suffixes `K`, `M`, `G` (optionally
/// followed by `B`), case-insensitive.
///
/// # Errors
///
/// Returns an error for non-numeric values or unknown suffixes.
pub fn parse_speed_spec(spec: &str) -> anyhow::Result<u64> {
  let spec = spec.trim();
  let (digits, multiplier) = match spec.chars().last() {
    Some(c) if c.is_ascii_alphabetic() => {
      let mut stripped = &spec[..spec.len() - 1];
      let mut base = c.to_ascii_uppercase();
      // Allow an optional trailing `B` (e.g. `10MB`): drop it and use the
      // preceding letter as the multiplier suffix.
      if base == 'B' {
        base = stripped
          .chars()
          .last()
          .map(|prev| prev.to_ascii_uppercase())
          .ok_or_else(|| anyhow::anyhow!("invalid speed `{spec}`: no number"))?;
        stripped = &stripped[..stripped.len() - 1];
      }
      let mult = match base {
        'K' => 1024u64,
        'M' => 1024 * 1024,
        'G' => 1024 * 1024 * 1024,
        _ => {
          return Err(anyhow::anyhow!(
            "invalid speed `{spec}`: suffix must be K, M, or G"
          ));
        }
      };
      (stripped, mult)
    }
    _ => (spec, 1),
  };
  let value: u64 = digits
    .trim()
    .parse()
    .map_err(|_| anyhow::anyhow!("invalid speed `{spec}`: not a number"))?;
  Ok(value * multiplier)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn env_from<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |k: &str| {
      pairs
        .iter()
        .find(|(key, _)| *key == k)
        .map(|(_, v)| (*v).to_owned())
    }
  }

  const SAMPLE: &str = r#"
[default]
output = "./downloads"
progress = true
max_speed = "20M"

[proxy]
url = "http://127.0.0.1:7890"
"#;

  #[test]
  fn test_parse_full_config() -> AnyResult<()> {
    let cfg: FileConfig = toml::from_str(SAMPLE)?;
    assert_eq!(Some("./downloads".to_owned()), cfg.default.output);
    assert_eq!(Some(true), cfg.default.progress);
    assert_eq!(Some("20M".to_owned()), cfg.default.max_speed);
    assert_eq!(Some("http://127.0.0.1:7890".to_owned()), cfg.proxy.url);
    Ok(())
  }

  #[test]
  fn test_parse_empty_config() -> AnyResult<()> {
    let cfg: FileConfig = toml::from_str("")?;
    assert!(cfg.default.output.is_none());
    assert!(cfg.proxy.url.is_none());
    Ok(())
  }

  #[test]
  fn test_parse_config_rejects_unknown_keys() {
    // Typos must fail loudly, not silently no-op.
    assert!(toml::from_str::<FileConfig>("[default]\nmax_sped = \"1M\"").is_err());
    assert!(toml::from_str::<FileConfig>("[unknown_section]\nx = 1").is_err());
  }

  #[test]
  fn test_resolve_file_only() -> AnyResult<()> {
    let cfg: FileConfig = toml::from_str(SAMPLE)?;
    let r = resolve(Some(&cfg), &env_from(&[]))?;
    assert_eq!(Some("./downloads".to_owned()), r.output);
    assert_eq!(Some(true), r.progress);
    assert_eq!(Some(20 * 1024 * 1024), r.max_speed_bytes);
    assert_eq!(Some("http://127.0.0.1:7890".to_owned()), r.proxy);
    Ok(())
  }

  #[test]
  fn test_resolve_env_beats_file() -> AnyResult<()> {
    let cfg: FileConfig = toml::from_str(SAMPLE)?;
    let r = resolve(
      Some(&cfg),
      &env_from(&[
        ("SIWI_DOWNLOAD_OUTPUT", "/elsewhere"),
        ("SIWI_DOWNLOAD_PROXY", "http://other:8080"),
        ("SIWI_DOWNLOAD_MAX_SPEED", "1K"),
      ]),
    )?;
    assert_eq!(Some("/elsewhere".to_owned()), r.output);
    assert_eq!(Some("http://other:8080".to_owned()), r.proxy);
    assert_eq!(Some(1024), r.max_speed_bytes);
    Ok(())
  }

  #[test]
  fn test_resolve_empty_env_value_counts_as_unset() -> AnyResult<()> {
    let cfg: FileConfig = toml::from_str(SAMPLE)?;
    let r = resolve(Some(&cfg), &env_from(&[("SIWI_DOWNLOAD_PROXY", "  ")]))?;
    // The blank env var must not wipe the config file proxy.
    assert_eq!(Some("http://127.0.0.1:7890".to_owned()), r.proxy);
    Ok(())
  }

  #[test]
  fn test_resolve_no_sources() -> AnyResult<()> {
    let r = resolve(None, &env_from(&[]))?;
    assert!(r.output.is_none());
    assert!(r.progress.is_none());
    assert!(r.max_speed_bytes.is_none());
    assert!(r.proxy.is_none());
    Ok(())
  }

  #[test]
  fn test_resolve_progress_bool_variants() -> AnyResult<()> {
    for (raw, expected) in [
      ("true", true),
      ("TRUE", true),
      ("1", true),
      ("yes", true),
      ("false", false),
      ("0", false),
      ("No", false),
    ] {
      let r = resolve(None, &env_from(&[("SIWI_DOWNLOAD_PROGRESS", raw)]))?;
      assert_eq!(Some(expected), r.progress, "input {raw}");
    }
    Ok(())
  }

  #[test]
  fn test_resolve_progress_invalid_errors() {
    let r = resolve(None, &env_from(&[("SIWI_DOWNLOAD_PROGRESS", "maybe")]));
    assert!(r.is_err());
  }

  #[test]
  fn test_resolve_max_speed_invalid_errors() {
    let r = resolve(None, &env_from(&[("SIWI_DOWNLOAD_MAX_SPEED", "fast")]));
    assert!(r.is_err());
  }

  #[test]
  fn test_parse_speed_spec_plain() {
    assert_eq!(1024, parse_speed_spec("1024").unwrap());
    assert_eq!(0, parse_speed_spec("0").unwrap());
  }

  #[test]
  fn test_parse_speed_spec_suffixes() {
    assert_eq!(1024, parse_speed_spec("1K").unwrap());
    assert_eq!(10 * 1024 * 1024, parse_speed_spec("10M").unwrap());
    assert_eq!(1024 * 1024 * 1024, parse_speed_spec("1G").unwrap());
  }

  #[test]
  fn test_parse_speed_spec_lowercase_and_b() {
    assert_eq!(500 * 1024, parse_speed_spec("500k").unwrap());
    assert_eq!(10 * 1024 * 1024, parse_speed_spec("10MB").unwrap());
    assert_eq!(3 * 1024 * 1024, parse_speed_spec("3Mb").unwrap());
  }

  #[test]
  fn test_parse_speed_spec_rejects_bad_suffix() {
    assert!(parse_speed_spec("10X").is_err());
  }

  #[test]
  fn test_parse_speed_spec_rejects_not_a_number() {
    assert!(parse_speed_spec("abc").is_err());
    assert!(parse_speed_spec("1.5M").is_err());
  }
}
