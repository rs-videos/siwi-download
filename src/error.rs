//! Error types and result aliases for siwi-download.
//!
//! This module provides type aliases for error handling using [`anyhow`],
//! which allows for ergonomic error propagation throughout the library.

/// The primary error type used by siwi-download.
///
/// This is an alias for [`anyhow::Error`], which provides a flexible
/// error type that supports context and backtraces.
pub type AnyError = anyhow::Error;

/// A result type for operations that may fail.
///
/// This is the standard result type used throughout siwi-download
/// for error handling.
///
/// # Example
///
/// ```rust,no_run
/// use siwi_download::error::AnyResult;
///
/// async fn download_file() -> AnyResult<()> {
///     // Operations that may fail
///     Ok(())
/// }
/// ```
pub type AnyResult<T> = anyhow::Result<T, AnyError>;
