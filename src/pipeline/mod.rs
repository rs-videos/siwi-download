//! **Unstable**: 3.0 Pipeline architecture (see `docs/design/30-pipeline.md`).
//!
//! A pipeline composes a [`Source`] (where bytes come from) with a [`Sink`]
//! (where they go), sharing the same transfer engine as
//! [`Download::download`](crate::download::Download::download): identical
//! lifecycle events, rate limiting, cancellation, and progress semantics.
//!
//! This module is gated behind the **`unstable`** feature while the 3.0 API
//! settles; it is additive over 2.x and may change in minor releases until
//! 3.0.0.
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
//! println!("{} bytes", report.bytes_written);
//! # Ok(())
//! # }
//! ```

pub mod filter;
pub mod runner;
pub mod source;

#[cfg(feature = "gzip")]
pub use filter::GunzipFilter;
pub use filter::{ChecksumFilter, Filter};
pub use runner::{Pipeline, PipelineOptions, PipelineReport};
pub use source::{FileSource, HttpSource, HttpSourceOptions, Source, SourceMetadata};

/// The pipeline's sink contract.
///
/// This is the 2.2 [`StreamSink`](crate::download::sink::StreamSink) under
/// its 3.0 name; the alias keeps both spellings valid during the transition.
pub use crate::download::sink::StreamSink as Sink;

/// Re-export of the built-in sinks that implement [`Sink`].
pub use crate::download::sink::{FileSink, HashSink, MemorySink, StdoutSink, TeeSink};
