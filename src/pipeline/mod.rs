//! Pipeline architecture (stable since 3.0; see
//! `docs/design/30-pipeline.md`).
//!
//! A pipeline composes a [`Source`] (where bytes come from) with a [`Sink`]
//! (where they go), sharing the same transfer engine as
//! [`Download::download`](crate::download::Download::download): identical
//! lifecycle events, rate limiting, cancellation, and progress semantics.
//!
//! # Example
//!
//! ```rust,no_run
//! use siwi_download::download::sink::MemorySink;
//! use siwi_download::pipeline::{HttpSource, Pipeline};
//!
//! # async fn example() -> siwi_download::error::AnyResult<()> {
//! let source = HttpSource::new("https://example.com/file.zip", Default::default())?;
//! let report = Pipeline::new(source, MemorySink::default()).run().await?;
//! println!("{} bytes", report.bytes_transferred);
//! # Ok(())
//! # }
//! ```

pub mod filter;
pub mod runner;
#[cfg(feature = "s3")]
pub mod s3;
pub mod source;

#[cfg(feature = "gzip")]
pub use filter::GunzipFilter;
pub use filter::{ChecksumFilter, Filter, TeeFilter};
pub use runner::{FilteredSink, Pipeline, PipelineOptions, PipelineReport};
#[cfg(feature = "s3")]
pub use s3::{S3Config, S3Sink, S3Source};
pub use source::{FileSource, HttpSource, HttpSourceOptions, Source, SourceMetadata};

/// The pipeline's sink contract.
///
/// This is the 2.2 [`StreamSink`](crate::download::sink::StreamSink) under
/// its 3.0 name; the alias keeps both spellings valid during the transition.
pub use crate::download::sink::StreamSink as Sink;

/// Re-export of the built-in sinks that implement [`Sink`].
pub use crate::download::sink::{FileSink, HashSink, MemorySink, StdoutSink, TeeSink};
