# Roadmap — siwi-download 2.x → 3.0

> 本文档规划 siwi-download 从 **2.0（2026-07）** 到 **3.0（约 2027-Q3）** 的迭代路线。
> 它是活的文档，每个 minor 发布后会回顾修订。
> 节奏：**每季度一个 minor（2.1 / 2.2 / 2.3 / 2.4）**，3.0 作为下一个大版本。

## 1. 3.0 的北极星

**「云原生场景下，Rust 生态最值得集成的下载抽象。」**

三条支柱：

| 支柱 | 含义 | 不做什么（明确边界） |
|---|---|---|
| **场景** | 专攻 Dockerfile / CI / 数据管道 / 边缘函数等云原生环境 | 不做 P2P（BT/磁力链） |
| **库优先** | `Download` 作为可组合 trait/抽象，CLI 只是它的渲染层 | 不做 GUI / TUI（CLI 即终点） |
| **可观测** | 一等公民的 metrics、事件流、hook | 不做自有 dashboard（走 OpenTelemetry） |

明确**不进入 3.0 愿景**的：
- ❌ 多连接分片下载（aria2 `-x16`）——红海赛道，单连接 + 续传已覆盖 95% 场景
- ❌ BitTorrent / 磁力链——独立生态，工作量大，偏离定位
- ❌ ratatui TUI——CLI 输出 + JSON 已足够，TUI 会拖慢核心迭代
- ❌ 自有 GUI / Electron 应用

## 2. 设计原则（贯穿所有 2.x）

这些原则约束每个版本的功能取舍：

1. **库优先，CLI 次之**：每个新能力必须先有干净的 Rust API，再考虑 CLI flag
2. **零配置可用，配置文件次之**：默认行为合理；复杂的配置才进文件
3. **向后兼容是硬约束**：2.x 内部不允许破坏性 API 变更；只能加 `#[deprecated]`
4. **依赖最小化**：避免引入大依赖；特性门控（feature flag）控制可选功能
5. **可观测优先于更快**：先让用户看清楚在发生什么，再谈优化
6. **失败可恢复**：任何 IO/网络操作必须支持重试 + 续传，不丢进度

## 3. 版本节奏总览

```
2026-Q3  2.0  ✅ 已发布（重构、断点续传、MSRV、CI）
2026-Q3  2.1  ✅ 已发布 v2.1.0（校验和、限速、条件请求、配置文件、环境变量）
2026-Q3  2.2  ✅ 已发布 v2.2.0（Hook 事件流、StreamSink、stream()、--stdout、gzip）
2026-Q3  2.3  ✅ 已发布 v2.3.0（DownloadQueue、并发上限、depends_on、--batch）
2026-Q4  2.4  ✅ 已发布 v2.4.0（metrics、访问日志、优雅退出、--dry-run）
2027-Q1  3.0  规划中：Pipeline 架构（Source → Filter → Sink）
```

每个 minor 约 12 周，典型拆分：6 周设计+开发、3 周测试+文档、3 周缓冲+发布。

---

## 4. 2.1 — 可靠性基础（2026-Q4）

**主题**：让下载结果可信、可控。当前 2.0 下完就是"Complete"，但没有验证文件是否损坏、无法限速、不支持 If-Modified 这类条件请求。

### 4.1 目标

| 能力 | 说明 | 优先级 |
|---|---|---|
| **校验和验证** | 下完后自动算 SHA-256 / MD5，与 URL 或 `--checksum` 提供的值对比 | P0 |
| **下载限速** | `--max-speed 10M` 控制带宽，避免打满网络 | P0 |
| **条件请求** | `If-Modified-Since` / `If-None-Match`，304 时跳过下载 | P1 |
| **配置文件** | `~/.config/siwi-download/config.toml` 持久化默认值 | P1 |
| **环境变量** | `SIWI_DOWNLOAD_PROXY` 等环境变量覆盖 | P2 |

### 4.2 API 草案

```rust
// 新增 checksum 模块
pub mod checksum {
    pub enum Algorithm { Sha256, Sha1, Md5 }
    pub fn parse_spec(spec: &str) -> AnyResult<(Algorithm, Vec<u8>)>;
}

// DownloadOptions 新增
impl DownloadOptions {
    pub fn set_checksum(&mut self, algo: checksum::Algorithm, expected: Vec<u8>) -> &mut Self;
    pub fn set_max_speed(&mut self, bytes_per_sec: u64) -> &mut Self;
    pub fn set_conditional(&mut self, last_modified: DateTime<Utc>, etag: Option<String>) -> &mut Self;
}

// DownloadReport 新增
pub struct DownloadReport {
    // ...existing fields...
    pub checksum_verified: Option<bool>,       // 校验结果
    pub not_modified: Option<bool>,            // 304 命中
    pub average_speed: Option<u64>,            // 平均速度 bytes/s
}
```

### 4.3 CLI

```bash
# 校验和
siwi-download https://example.com/file.iso \
    --checksum sha256:abc123... \
    -P

# 限速
siwi-download https://example.com/file.iso --max-speed 10M

# 仅当远端更新才下载
siwi-download https://example.com/data.json --if-modified
```

### 4.4 配置文件示例

```toml
# ~/.config/siwi-download/config.toml
[default]
output = "./downloads"
progress = true
max_speed = "20M"           # 字符串，支持 K/M/G 后缀

[proxy]
url = "http://127.0.0.1:7890"
```

### 4.5 验收标准

- [ ] 单元测试覆盖 checksum 三种算法
- [ ] 集成测试：故意返回错误 checksum 时 `DownloadReport.checksum_verified == Some(false)` 且 `download_status == Error`
- [ ] 限速实测：`--max-speed 1M` 下网络监控显示 ≈1 MB/s
- [ ] 配置文件优先级：CLI flag > 环境变量 > 配置文件 > 内置默认

### 4.6 依赖

- `sha2`、`md-5`、`sha1`（RustCrypto 系列，纯 Rust，零额外开销）
- `tokio-util` 的 `RateLimit`（已有，不增加依赖）

**估算**：~6 周开发 + ~2 周打磨 = **8 人周**

---

## 5. 2.2 — Hook 与事件流（2027-Q1）⭐ 核心版本

**主题**：把"下载"从黑盒变成可观测、可介入的过程。这是 3.0 的"流式架构"的预演。

### 5.1 设计目标

用户能在下载的**任意阶段**介入：
- 开始前：决定是否真的下载
- 收到响应头：读取 metadata、改写存储路径
- 每个 chunk 写入前后：做流式处理（边下边解压、边下边验）
- 完成/失败：触发通知

### 5.2 核心 trait 设计

```rust
/// 下载生命周期事件
#[derive(Debug, Clone)]
pub enum DownloadEvent<'a> {
    /// 即将发起请求，可修改 options
    BeforeRequest { url: &'a str, options: &'a mut DownloadOptions },
    /// 收到响应头，可改写存储路径或放弃
    HeadersReceived { url: &'a str, status: u16, headers: &'a HeaderMap },
    /// 一个 chunk 已下载，尚未写入磁盘
    ChunkReceived { seq: u64, bytes: &'a [u8] },
    /// 一个 chunk 已写入磁盘
    ChunkWritten { seq: u64, offset: u64, len: usize },
    /// 整体进度更新
    Progress { downloaded: u64, total: Option<u64> },
    /// 完成
    Complete { report: &'a DownloadReport },
    /// 失败
    Error { error: &'a AnyError, partial: bool },
}

/// Hook trait，用户实现它来介入下载
pub trait DownloadHook: Send + Sync {
    fn on_event(&self, event: DownloadEvent<'_>) -> AnyResult<()>;
}

/// 流式 sink trait，用于"边下边处理"
#[async_trait]
pub trait StreamSink: Send + Sync {
    async fn write_chunk(&mut self, chunk: &Bytes) -> AnyResult<()>;
    async fn finalize(self: Box<Self>) -> AnyResult<()>;
}
```

### 5.3 API 草案

```rust
impl Download {
    /// 带事件流的下载
    pub async fn download_with_events(
        &self,
        url: impl AsRef<str>,
        options: DownloadOptions,
        events: impl DownloadHook,
    ) -> AnyResult<DownloadReport>;

    /// 流式下载，不落盘，直接交给 sink
    pub async fn stream(
        &self,
        url: impl AsRef<str>,
        options: DownloadOptions,
        sink: impl StreamSink,
    ) -> AnyResult<DownloadReport>;
}

// DownloadOptions 新增
impl DownloadOptions {
    pub fn add_hook(&mut self, hook: Arc<dyn DownloadHook>) -> &mut Self;
    pub fn set_sink(&mut self, sink: Arc<dyn StreamSink>) -> &mut Self;
}
```

### 5.4 内置 StreamSink 示例

```rust
// 文件 sink（默认行为）
pub struct FileSink { path: PathBuf, /* ... */ }

// 解压 sink：边下边解压
pub struct GunzipSink<inner: StreamSink> { inner: inner }

// hash sink：边下边算 hash
pub struct HashSink { algo: Algorithm, hasher: Sha256 }

// tee sink：同时写两个 sink
pub struct TeeSink<A: StreamSink, B: StreamSink> { a: A, b: B }
```

### 5.5 CLI 事件

```bash
# 边下边解压到目录
siwi-download https://example.com/data.tar.gz \
    --pipe "tar -xz -C ./data"

# 下载完触发脚本
siwi-download https://example.com/file.iso \
    --on-complete "./notify.sh {file_path}"

# 流式下载到 stdout
siwi-download https://example.com/data.json --stdout | jq .
```

### 5.6 验收标准

- [ ] `stream()` API 不落盘，内存占用恒定（实测下 1GB 文件，RSS < 100MB）
- [ ] `GunzipSink` 实测：`.tar.gz` 文件下载完即可用，无需先落盘再解压
- [ ] Hook 可中断下载：`BeforeRequest` 返回错误时下载立即停止
- [ ] 内置至少 3 个 StreamSink（File/Hash/Tee）
- [ ] 事件有序：测试覆盖事件顺序（BeforeRequest → Headers → Chunk* → Complete）

### 5.7 风险

- **性能**：每个 chunk 都触发 hook 可能影响吞吐。缓解：批量 hook、`Progress` 事件限频（默认 100ms 一次）
- **API 复杂度**：`DownloadHook` 是 trait object 还是泛型？倾向 trait object（`Arc<dyn DownloadHook>`）保兼容

**估算**：~10 周开发 + ~4 周打磨 = **14 人周**（这是整个路线图最重的版本）

---

## 6. 2.3 — 多任务编排（2027-Q2）

**主题**：从"下单个文件"升级到"管理一组下载任务"。复用 2.2 的事件流。

### 6.1 目标

| 能力 | 说明 |
|---|---|
| **任务队列** | 一组 URL 顺序下载 |
| **并发上限** | `--concurrent 4` 同时跑 4 个 |
| **依赖关系** | B 等 A 完成后才下 |
| **恢复中断的批次** | 进程崩溃后，重启继续未完成任务 |
| **manifest 格式** | YAML/TOML 描述批量任务 |

### 6.2 API 草案

```rust
pub struct DownloadQueue {
    queue: VecDeque<DownloadTask>,
    max_concurrent: usize,
    state_file: Option<PathBuf>,
}

pub struct DownloadTask {
    pub url: String,
    pub options: DownloadOptions,
    pub depends_on: Vec<String>,  // 其他 task 的 ID
    pub id: String,
}

impl DownloadQueue {
    pub fn new(max_concurrent: usize) -> Self;
    pub fn push(&mut self, task: DownloadTask);
    pub async fn run(&mut self) -> AnyResult<Vec<DownloadReport>>;
    pub fn save_state(&self, path: &Path) -> AnyResult<()>;
    pub fn load_state(path: &Path) -> AnyResult<Self>;
}
```

### 6.3 Manifest 格式

```yaml
# batch.yaml
concurrent: 4
state_file: ./download-state.json

tasks:
  - id: model
    url: https://example.com/model.bin
    output: ./models
  - id: dataset
    url: https://example.com/dataset.tar.gz
    output: ./data
    depends_on: [model]   # 等 model 下完
  - id: extract
    pipe: "tar -xz -C ./data"
    depends_on: [dataset]
```

```bash
siwi-download batch batch.yaml
```

### 6.4 验收标准

- [ ] `concurrent=4` 时 4 个文件并行，其他排队
- [ ] `depends_on` 形成环时报错（拓扑排序检测）
- [ ] 进程 SIGINT 后，`state_file` 完整保存；重启能继续
- [ ] 单个任务失败不影响其他独立任务
- [ ] CLI 输出聚合报告（成功 N、失败 M）

**估算**：~8 人周

---

## 7. 2.4 — 可观测性与生产就绪（2027-Q3）

**主题**：让 siwi-download 在生产环境用得放心。

### 7.1 目标

| 能力 | 说明 |
|---|---|
| **Metrics** | 暴露 Prometheus 文本格式或 OTel metrics |
| **访问日志** | 每次下载写 JSON 行日志（可配 stdout/file） |
| **访问日志轮转** | 内置或推荐 `logrotate` |
| **优雅退出** | SIGTERM 触发完成当前 chunk → 落盘 state → 退出 |
| **dry-run** | `--dry-run` 只发 HEAD，不下载，输出报告 |
| **退出码规范** | `0` 成功 / `2` 参数错 / `10` 校验失败 / `11` 网络失败 / `12` 磁盘满 |

### 7.2 Metrics 指标集

```
siwi_downloads_total{status="complete"} 42
siwi_downloads_total{status="error"} 3
siwi_download_bytes_total 1234567890
siwi_download_duration_seconds_bucket{le="1"} 5
siwi_download_retries_total 12
siwi_download_resume_bytes_total 987654
```

暴露方式：
- CLI: `--metrics-file ./metrics.prom`
- 库: `metrics::Registry` 可注入

### 7.3 优雅退出

```rust
use tokio::signal;

let download = Download::new("./downloads");
let report = tokio::select! {
    r = download.download(url, options) => r,
    _ = signal::ctrl_c() => {
        // 完成当前 chunk → flush → 保存 state → 返回 Resume 状态
        download.graceful_shutdown().await
    }
};
```

### 7.4 验收标准

- [ ] Prometheus 可抓取 `--metrics-file` 输出
- [ ] SIGINT 触发 graceful shutdown 时，已写入字节不丢
- [ ] 退出码符合规范文档
- [ ] `--dry-run` 不实际写入文件

**估算**：~6 人周

---

## 8. 3.0 — 流式架构里程碑（2027-Q4）

3.0 不是"功能大爆炸"，而是**架构重构的收口**：把 2.2 引入的 hook + sink 抽象升级为核心架构，让 siwi-download 从"下载器"变成"下载管道"。

### 8.1 3.0 核心变更

#### 8.1.1 Trait 抽象上升为公共契约

```rust
/// 下载源（URL / S3 / IPFS / 本地路径，都实现它）
#[async_trait]
pub trait Source {
    async fn metadata(&self) -> AnyResult<SourceMetadata>;
    async fn chunks(&self) -> AnyResult<Box<dyn ChunkStream>>;
}

/// 下载目的（文件 / stdout / 内存 / S3 上传）
#[async_trait]
pub trait Sink {
    async fn write_chunk(&mut self, chunk: Bytes) -> AnyResult<()>;
    async fn finalize(self: Box<Self>) -> AnyResult<()>;
}

/// 下载管道：Source → [Filter] → Sink
pub struct Pipeline {
    source: Box<dyn Source>,
    filters: Vec<Box<dyn Filter>>,
    sink: Box<dyn Sink>,
}

impl Pipeline {
    pub async fn run(&mut self) -> AnyResult<DownloadReport>;
}
```

这让 siwi-download 能：
- 从 S3 下到本地文件（`S3Source → FileSink`）
- 从 HTTP 下到 S3（`HttpSource → S3Sink`）—— **完全 bypass 本地磁盘**
- HTTP 下载中实时 GZIP 解压（`HttpSource → GunzipFilter → FileSink`）

#### 8.1.2 破坏性 API 变更（允许，因为是 major）

- `Download` struct 可能被 `Pipeline` 替代（保留 `Download` 作为 v2 兼容包装器，标 `#[deprecated]`）
- `DownloadOptions` 拆成 `SourceOptions` / `SinkOptions` / `PipelineOptions`
- `DownloadReport` 扩展为 `PipelineReport`，含每阶段耗时

#### 8.1.3 模块重组

```
src/
├── source/
│   ├── mod.rs
│   ├── http.rs      # 现在的 download 逻辑迁移过来
│   ├── s3.rs        # 新增（feature = "s3"）
│   └── file.rs      # 新增
├── sink/
│   ├── mod.rs
│   ├── file.rs
│   ├── stdout.rs
│   └── s3.rs        # feature = "s3"
├── filter/
│   ├── mod.rs
│   ├── gunzip.rs
│   ├── checksum.rs
│   └── tee.rs
├── pipeline.rs      # Source → Filter* → Sink
└── ...
```

### 8.2 3.0 验收标准

- [ ] **三种 Source × 三种 Sink 矩阵**都能跑通（HTTP/S3/File 互转）
- [ ] HTTP → S3 直传不落本地磁盘（实测内存 < 50MB 下 5GB 文件）
- [ ] 提供 `siwi-download v2` 兼容模块，标 `#[deprecated]`，2.x 代码加一行能编译过
- [ ] 性能不退化：HTTP → File 对比 2.4 基线，吞吐降低 < 5%

### 8.3 3.0 不做的事

- 不做 BT / 磁力链（永远不做，见第 1 节）
- 不做 GUI / TUI
- 不做多连接分片下载
- 不引入 async trait 之外的运行时抽象（不写自己的 Future、不强行集成 bevy_ecs 之类的）

**估算**：~16 人周（含兼容层 + 完整文档 + 三种 source/sink 实现）

---

## 9. 工作量汇总与优先级

| 版本 | 主题 | 人周 | 累计 | 优先级 |
|---|---|---|---|---|
| 2.1 | 可靠性基础 | 8 | 8 | P0 |
| 2.2 | Hook 与事件流 ⭐ | 14 | 22 | P0 |
| 2.3 | 多任务编排 | 8 | 30 | P1 |
| 2.4 | 可观测性 | 6 | 36 | P1 |
| 3.0 | 流式架构 | 16 | 52 | P0 |
| **总计** | | **52 人周** | | ~13 人月 |

> 52 人周按每季度投入 13 人周算 = **每季度 1 人全职**，符合"季度 minor"节奏。

### 优先级原则

- **P0** = 不做就无法实现 3.0 愿景，必须按时完成
- **P1** = 显著提升价值，但可延后一个版本
- **P2** = 锦上添花，按精力决定

## 10. 风险与缓解

| 风险 | 概率 | 影响 | 缓解 |
|---|--- 性能 | Hook 每 chunk 触发影响吞吐 | 中 | 中 | 批量 hook、事件限频、基准测试守门 |
| API 过度设计 | 中 | 高 | 每个新 trait 先发 unstable feature flag，收集反馈再稳定 |
| 2.2 工作量超预期 | 中 | 高 | Hook 和 StreamSink 拆分独立交付，Hook 先发 |
| 3.0 兼容层维护成本 | 中 | 中 | `#[deprecated]` + 一个版本的过渡期，4.0 删 |
| 依赖膨胀（s3/加密） | 高 | 中 | feature flag 隔离，默认不编译 |
| 维护者精力不足 | 高 | 高 | 每个版本预留 3 周缓冲，宁可延期不砍测试 |

## 11. 版本兼容矩阵

| 版本 | 兼容性 | 升级建议 |
|---|---|---|
| 2.x → 2.(x+1) | 向后兼容 | 直接升级 |
| 2.4 → 3.0 | **破坏性** | 使用 `siwi_download::v2` 兼容模块平滑迁移 |
| 3.x → 4.0 | 破坏性 | 删除 v2 兼容层 |

**SemVer 承诺**：
- 2.x 内部，公开 API 只能加，不能改/删
- 标记 `#[deprecated]` 的 API 至少保留 2 个 minor 才能删
- MSRV 升级 = minor 版本升级（不算破坏性，但 CHANGELOG 必须标）

## 12. 决策日志（ADR 摘要）

记录关键的设计决策，避免反复讨论：

| ID | 决策 | 理由 | 日期 |
|---|---|---|---|
| 001 | 不做多连接分片下载 | 单连接 + 续传覆盖 95% 场景；赛道拥挤 | 2026-07 |
| 002 | 不做 BT/磁力链 | 独立生态，偏离云原生定位 | 2026-07 |
| 003 | Hook 用 trait object 而非泛型 | API 稳定，便于动态注册 | 2026-09（已实现） |
| 004 | 3.0 用 Pipeline 架构，非嵌入式 | Source/Sink 抽象解锁云原生场景 | 2027-Q4（计划） |
| 005 | `DownloadHook::on_event` 是同步方法 | 保持 trait 对象安全（无需 async_trait）；`CommandHook` 等阻塞型 hook 文档注明运行在下载任务上。异步 hook 推迟到 3.0 Pipeline（async trait 届时更成熟） | 2026-09 |

## 13. 如何贡献

- 在 [Issues](https://github.com/rs-videos/siwi-download/issues) 标签 `roadmap` / `2.1` / `2.2` 下认领任务
- 任何对路线图本身的建议，开 issue 标 `roadmap-discussion`
- 大功能（P0）请先开 design issue 讨论，不要直接 PR

## 14. 反驳与重新讨论

这份路线图是基于 2026-Q3 的认识。如果出现以下情况，应重新讨论：

- Rust async ecosystem 出现重大变化（如 trait async 稳定、新 runtime 主导）
- 用户反馈强烈要求 BT/分片下载（>10 个独立 issue）
- crates.io 同类项目（如 reqwest 衍生下载器）发布了同质化功能
- 维护者投入精力显著变化

---

_最后更新：2026-07-21 · 下次回顾：2.1 发布后_
