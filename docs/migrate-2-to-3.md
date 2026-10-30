# Migrating from 2.x to 3.0

3.0 stabilizes the **Pipeline** API (Source → Filter → Sink) that shipped
behind the `unstable` feature in 2.5–2.7. The headline: **no 2.x API was
removed**. Existing 2.x code compiles and behaves unchanged — migration is
only required if you opted into `features = ["unstable"]`.

## TL;DR

| You use… | Action needed |
|---|---|
| 2.x API only (`Download`, `DownloadOptions`, `DownloadQueue`, hooks…) | **None** — recompile |
| `features = ["unstable"]` (pipeline 2.5–2.7) | Remove the feature and rename `PipelineReport` fields (table below) |
| `features = ["gzip"]` | None |
| `features = ["s3"]` | None (feature remains; no longer implies `unstable`) |

## 1. Feature flags

The `unstable` and `docs` features are gone; the pipeline is always
available.

```diff
 [dependencies]
-siwi-download = { version = "2.7", features = ["unstable"] }
+siwi-download = "3"
```

`gzip` (GunzipSink/GunzipFilter) and `s3` (S3Source/S3Sink) remain
opt-in features.

## 2. PipelineReport field renames

The 2.5–2.7 field names were provisional (that is what `unstable` meant).
The 3.0 names state what the numbers are:

| 2.x (`unstable`) | 3.0 | Meaning |
|---|---|---|
| `bytes_written` | `bytes_transferred` | bytes moved by this run |
| `resumed_from` | `bytes_skipped` | bytes skipped by the resume offset |
| `total` | `content_length` | full payload size, when the source reports it |

Everything else (`location`, `status`, `etag`, `last_modified`,
`cancelled`) is unchanged, as are `Pipeline`, `PipelineOptions`, `Source`,
`Sink`, `Filter`, and all built-in sources/filters/sinks.

## 3. Nothing else moved

`Download`, `DownloadOptions`, `DownloadReport`, `DownloadQueue`,
`DownloadHook`, `Metrics`, `AccessLogHook`, `Download::probe`, and the CLI
flags are untouched. 2.x examples in the README still build as written.

## 4. Adopting the pipeline (optional)

The pipeline composes a source with a sink and runs the same transfer
engine (and therefore the same hooks, rate limiting, cancellation, and
progress semantics) as `Download::download`:

```rust
use siwi_download::download::sink::MemorySink;
use siwi_download::pipeline::{HttpSource, Pipeline, PipelineOptions};

let report = Pipeline::new(
    HttpSource::new("https://example.com/file.zip", Default::default())?,
    MemorySink::default(),
)
.options(PipelineOptions::default())
.run()
.await?;
println!("{} bytes", report.bytes_transferred);
```

### Filters

```rust
use siwi_download::download::sink::MemorySink;
use siwi_download::download::Algorithm;
use siwi_download::pipeline::{ChecksumFilter, Filter, Pipeline};

let expected = "b94d27..."; // sha256 hex of the payload
let report = Pipeline::new(source, MemorySink::default())
    .filter(Box::new(ChecksumFilter::new(Algorithm::Sha256, expected)) as Box<dyn Filter>)
    .run()
    .await?;
```

A filter error aborts the pipeline **before** the sink finalizes, so a
checksum mismatch never produces a "complete" sink.

With the `gzip` feature, `GunzipFilter` decompresses in flight;
`TeeFilter` keeps a side copy (e.g. the raw compressed bytes).

### Using filters with `Download::stream`

`FilteredSink` adapts the existing streaming API without new core surface:

```rust
use siwi_download::pipeline::FilteredSink;

let mut sink = FilteredSink::new(MemorySink::default())
    .push(Box::new(GunzipFilter::new()) as Box<dyn Filter>);
download.stream(url, options, &mut sink).await?;
```

## 5. S3 (feature `s3`)

No AWS SDK involved: requests are signed with SigV4 by an implementation
verified against official AWS test vectors.

```rust
use siwi_download::pipeline::{S3Config, S3Sink, S3Source};

let config = S3Config {
    endpoint: "https://s3.us-east-1.amazonaws.com".into(),
    region: "us-east-1".into(),
    bucket: "my-bucket".into(),
    access_key: std::env::var("AWS_ACCESS_KEY_ID")?,
    secret_key: std::env::var("AWS_SECRET_ACCESS_KEY")?,
    session_token: None,
    path_style: false,
};
// Download: S3Source → any sink
let source = S3Source::new(config.clone(), "data/file.bin")?;
// Upload: any source → S3Sink (single PutObject; 64 MiB memory spool cap)
let sink = S3Sink::new(config, "uploads/file.bin")?;
```

`S3Sink` spools to memory and performs one `PutObject` with an exact
payload hash; exceeding `max_buffer()` fails rather than spilling to disk.
Multipart upload is deferred to a later minor.

## 6. Why 3.0 if nothing breaks?

SemVer major here signals the **stability commitment** for the pipeline
surface and the removal of the `unstable` opt-in, and it is a breaking
change for anyone who was using the unstable fields. There is no
`v2` compatibility module because 3.0 removes no 2.x API (see ADR-010 in
`docs/design/30-pipeline.md`).

## Compatibility policy from 3.0 on

- 3.x: public API is additive only; anything deprecated stays for at least
  two minor releases.
- `gzip` / `s3` remain opt-in features; no default-feature growth.
