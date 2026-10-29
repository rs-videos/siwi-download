# RFC: 3.0 Pipeline Architecture — Source → Filter → Sink

- **Status**: Draft for discussion
- **Target version**: 3.0.0
- **Related ADRs**: 001 (no sharding), 002 (no P2P), 003 (trait-object hooks), 004 (pipeline at 3.0), 005 (sync hooks)
- **Discuss**: open an issue tagged `roadmap-discussion` referencing this file

## 1. Summary

3.0 collapses `Download::download` / `Download::stream` into a single
composable pipeline:

```text
Source (where bytes come from)  →  Filter* (transformations)  →  Sink (where bytes go)
```

The headline capability this unlocks is **HTTP → S3 direct transfer** with
zero local disk usage, plus a `Source`/`Sink` matrix (HTTP/S3/File in any
combination) that cloud-native and data-pipeline users compose freely.

## 2. Goals

1. **One execution engine.** `download()` and `stream()` become thin
   wrappers over the pipeline; their behavior (hooks, rate limiting,
   cancellation, events) is defined once.
2. **S3 support, feature-gated.** `s3` feature adds `S3Source`/`S3Sink`
   (AWS SDK is heavy — it must never be a default dependency).
3. **Full backwards-compat layer.** `siwi_download::v2` re-exports the 2.x
   API with `#[deprecated]` markers; 2.x users migrate by changing one
   import line.
4. **No throughput regression.** Pipeline overhead vs 2.4's direct write
   path must stay under 5% on the HTTP → File benchmark.

## 3. Non-Goals (per ADR-001/002)

- No multi-connection sharding, no BitTorrent/magnet.
- No TUI/GUI.
- No new async runtime abstractions; tokio stays the only runtime.
- `async-trait` crate is not introduced; traits use native
  async-fn-in-trait (RPITIT) as `StreamSink` already does. Where dyn
  dispatch is needed, we box explicitly (`Box<dyn Source>`).

## 4. Trait Design

### 4.1 Source

```rust
/// Metadata about the thing a Source will produce.
pub struct SourceMetadata {
    pub content_length: Option<u64>,
    pub etag: Option<String>,
    pub last_modified: Option<DateTime<Utc>>,
    pub accepts_range: bool,
}

#[async_trait_like] // native RPITIT, Send futures
pub trait Source: Send {
    /// Probe without transferring (HEAD / S3 HeadObject).
    async fn metadata(&self) -> AnyResult<SourceMetadata>;

    /// The byte stream. Implementations should be resumable-aware:
    /// `range_from` > 0 asks the source to start mid-object when it can.
    async fn chunks(&mut self, range_from: u64)
        -> AnyResult<impl Stream<Item = AnyResult<Bytes>> + Send + '_>;
}
```

First implementations:

| Source | Feature | Notes |
|---|---|---|
| `HttpSource` | default | Extracted from today's `download()`; owns the HEAD-probe + retry logic |
| `S3Source` | `s3` | HeadObject/GetObject; multipart not needed (no sharding) |
| `FileSource` | default | Local file → useful for tests and file→S3 uploads |

### 4.2 Sink

`StreamSink` graduates to the pipeline contract — it is already correct
after 2.2 (native async trait, generic dispatch, `finalize`). Changes:

```rust
pub trait Sink: Send {
    async fn write_chunk(&mut self, chunk: &[u8]) -> AnyResult<()>;
    async fn finalize(&mut self) -> AnyResult<()>;
}
```

- `StreamSink` 2.2 becomes an alias of `Sink` during the compat window.
- `FileSink` gains an optional `truncate_on_range_restart` behavior: when
  the source ignores a resume range, today's append mode would corrupt;
  the pipeline's coordinator owns that policy instead of the sink.

### 4.3 Filter

```rust
pub trait Filter: Send {
    async fn transform(&mut self, chunk: &[u8], out: &mut Vec<u8>)
        -> AnyResult<()>;
    async fn flush(&mut self, out: &mut Vec<u8>) -> AnyResult<()>;
}
```

- `GunzipFilter` — extracted from 2.2's `GunzipSink` (the RFC-1952 framing
  code moves verbatim).
- `ChecksumFilter` — replaces post-hoc `verify()`; a mismatch aborts the
  pipeline before `Sink::finalize`.
- `TeeFilter` — fan-out, replacing `TeeSink` composition.
- Buffers: filters must document their memory ceiling; the runner enforces
  a max inter-filter buffer (default 16 MiB) and errors beyond it.

### 4.4 Pipeline runner

```rust
pub struct Pipeline { /* source, filters, sink, options */ }

impl Pipeline {
    pub fn new(source: impl Source + 'static, sink: impl Sink + 'static) -> Self;
    pub fn filter(mut self, f: impl Filter + 'static) -> Self;
    pub fn options(mut self, o: PipelineOptions) -> Self;   // hooks, cancel, max_speed
    pub async fn run(self) -> AnyResult<PipelineReport>;
}
```

`PipelineReport` extends today's `DownloadReport` with per-stage timings
(probe/transfer/finalize) — feed it into the 2.4 `Metrics`/`AccessLogHook`
unchanged.

### 4.5 Hooks, events, cancellation — unchanged

`DownloadEvent`/`DownloadHook` carry over as-is; the runner emits the same
sequence, adding `Stage { name, started, finished }` metadata to
`HeadersReceived`-equivalent events where useful. `cancel: Arc<AtomicBool>`
stays the cancellation mechanism (checked between chunks/filters).

## 5. Migration Path

```rust
// 2.x (continues to compile via the compat module)
use siwi_download::download::{Download, DownloadOptions};

// 3.x
use siwi_download::pipeline::{HttpSource, FileSink, Pipeline, PipelineOptions};

let report = Pipeline::new(HttpSource::new(url), FileSink::create(path).await?)
    .options(PipelineOptions::from(old_options))
    .run()
    .await?;
```

- `siwi_download::v2` module re-exports everything from 2.4 with
  `#[deprecated]`; removed no earlier than 4.0.
- `docs/migrate-2-to-3.md` ships with the release, one section per
  migration pattern (single download / streaming / queue).

## 6. Milestones (each independently shippable behind an unstable feature)

| # | Scope | Est. |
|---|---|---|
| M1 | `Source`/`Sink` traits + `HttpSource`/`FileSink` + runner; `download()` refactored onto it; benchmarks green | 5 wks |
| M2 | `Filter` trait + Gunzip/Checksum/Tee migration; `stream()` refactored onto it | 3 wks |
| M3 | `s3` feature: S3Source/S3Sink; HTTP→S3 e2e test with minio in CI | 4 wks |
| M4 | compat module `v2`, migration guide, docs, benchmark report | 2 wks |
| — | buffer + release hardening | 2 wks |

Total ≈ 16 person-weeks, matching the ROADMAP estimate.

## 7. Risks & Mitigations

| Risk | Mitigation |
|---|---|
| Throughput regression from double-buffering at filter boundaries | M1 benchmark gate (< 5%); filters write directly into a reused scratch buffer |
| AWS SDK dependency weight | `s3` feature only; CI runs default-feature builds to catch accidental leakage |
| Event-stream semantic drift during refactor | 2.2/2.3 integration tests (mock servers) must pass unchanged against the pipeline engine — they are the spec |
| Over-abstracted traits | No `Source`/`Sink` method beyond what M1/M3 consumers actually use; YAGNI review before M2 |

## 8. Open Questions

1. Should `FileSource` support sparse/`seek`-style random access in M1, or
   is sequential-only acceptable? (Lean: sequential-only.)
2. `PipelineReport` field naming — keep 2.x `file_size`/`range_from`
   vocabulary or rename to `bytes_total`/`bytes_skipped`? (Lean: rename in
   3.x since the compat layer absorbs the old names.)
3. Does the queue (`DownloadQueue`) move onto `Pipeline` in 3.0 or stay on
   the compat shim? (Lean: move in 3.0; it consumes only the runner.)

## 9. Decision Log Update

On approval, add:

| ADR | Decision |
|---|---|
| 006 | 3.0 pipeline uses native RPITIT traits; `Box<dyn>` only at construction sites |
| 007 | `s3` support is feature-gated; AWS SDK never a default dependency |
| 008 | Events/hooks semantic compatibility is enforced by treating 2.2+ integration tests as the executable spec |
