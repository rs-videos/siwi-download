# siwi-download 2.x 全记录：五个月，五个版本，从重构到生产就绪

> 仓库：https://github.com/rs-videos/siwi-download · 当前版本 v2.4.0 · MIT
> 前篇：[《拆解 siwi-download：Rust 异步下载器是怎么炼成的》](./03-deep-dive.md)

2026 年 7 月，siwi-download 2.0 发布。当时它只是一个"能续传的下载器"。五个月后，2.4 落地，它已经是一套**可观测、可编排、可流式处理**的下载基础设施。

这篇文章按版本线回顾 2.x 的完整演进，每个版本讲清楚"为什么做、做了什么、学到什么"，最后给出 3.0 的方向预告。如果你想找一个**从零到生产就绪的真实 Rust 项目样本**，这篇就是它的完整档案。

## 版本时间线

```
2026-07  2.0.0  重构与地基：模块化、API 简化、正确性修复
2026-09  2.1.0  可靠性：校验和、限速、条件请求、配置分层
2026-09  2.2.0  流式化：Hook 事件流、StreamSink、边下边处理
2026-09  2.3.0  编排：DownloadQueue、依赖图、--batch
2026-09  2.4.0  可观测：Prometheus metrics、访问日志、优雅退出
```

> 实际节奏远快于路线图排期（原计划每季度一个 minor）——这得益于"每个版本一个主题"的纪律和先写验收标准再动手的习惯。

---

## 2.0 — 把地基打对

2.0 不加功能，专注把 1.x 的债清掉。三件事最值得说：

### 1. 修掉一个"永远不会重试"的重试逻辑

1.x 的 HEAD 重试代码里有一行：

```rust
|| status >= MAX_HEAD_REQUEST_RETRIES  // 常量 = 5
```

重试次数常量被误用为 HTTP 状态码阈值——HTTP 状态码都 ≥ 100，条件永远为真，**循环第一次就 break**。2.0 把"状态判断"和"重试计数"分离成两个独立条件。教训：常量命名语义化（`MAX_RETRY_COUNT`）能从根上避免这类误用。

### 2. API 去掉了所有 `Cow<'a, str>`

1.x 的公共 API 带着生命周期参数 `DownloadOptions<'a>`，但调用点几乎全是 owned 值——`Cow` 的零拷贝收益约等于零，代价是生命周期污染整个使用方代码。2.0 全部改为 `String` + `impl Into<String>`，调用方代码立刻干净。

**原则：`Cow` 是解析器/序列化器高频路径的工具，不是配置对象的装饰品。**

### 3. 文件 I/O 全面 async 化

`is_file`/`is_dir`/`get_file_size` 从同步 `Path::metadata()` 改为 `tokio::fs::metadata`——在 tokio runtime 里一个同步 `metadata()` 就能拖慢所有并发任务。同时去掉了无意义的 `AnyResult` 包装（软检查本来就不该失败）。

另外：启用 `clippy::pedantic`（至今零告警）、声明 MSRV 1.85、补 LICENSE、HEAD 请求加 60s/10s 超时、CLI 无参数时打印完整 help。

---

## 2.1 — 让下载结果可信

主题：**下载完成了，但你怎么知道文件没坏？**

### 校验和验证

```bash
siwi-download https://example.com/file.iso --checksum sha256:abc123...
```

新增 `download::checksum` 模块：`Algorithm`（SHA-256/SHA-1/MD5）、`parse_spec`（`algo:hex` 语法校验）、流式 `hash_file`（64KiB 缓冲，多 GB 文件不 OOM）。**关键细节：校验覆盖整个磁盘文件**——续传场景下不只是校验新下的尾巴。

不匹配时 `download_status: "Error"` + 退出码 10，脚本可分支。

### 限速与条件请求

```bash
siwi-download URL --max-speed 10M      # 1024 进制 K/M/G
siwi-download URL --if-modified        # 304 时跳过，不 touch 本地文件
```

`--if-modified` 有个容易漏的实现细节：**304 响应必须在打开目标文件之前返回**——append 模式打开就会 touch mtime，破坏下一次 `If-Modified-Since` 的判断依据。

### 配置分层

```bash
# 优先级：CLI flag > 环境变量 > 配置文件 > 内置默认
export SIWI_DOWNLOAD_PROXY=http://127.0.0.1:7890
siwi-download URL                     # 自动读 ~/.config/siwi-download/config.toml
```

配置文件用 `deny_unknown_fields`——写错字（`max_sped`）会响亮报错而不是静默无效。`config.rs` 是 bin-only 模块，`dirs`/`toml` 依赖不进库的公共 API 面。

**顺带修了一个潜伏 bug**：tracing 日志原本写 stdout，`--json` 输出被污染，`| jq` 直接解析失败。现在日志固定走 stderr。

---

## 2.2 — 从黑盒到管道

主题：**下载过程能不能被观察、被介入？** 这是整个 2.x 最核心的架构演进，也是 3.0 的地基。

### Hook 事件流

```rust
pub enum DownloadEvent {
    BeforeRequest { .. }, HeadersReceived { .. },
    ChunkWritten { .. }, Progress { .. },
    Retry { attempt, status },        // 2.4 加入
    Complete { .. }, Error { .. },
}

pub trait DownloadHook: Send + Sync {
    fn on_event(&self, event: DownloadEvent<'_>) -> AnyResult<()>;
}
```

hook 返回 Err 即中断下载。`Progress` 限频 100ms 防止慢 hook 拖垮吞吐。内置 `CommandHook` 让 CLI 一行命令挂上完成回调：

```bash
siwi-download URL --on-complete 'notify-send "done: $SIWI_FILE_PATH"'
```

上下文通过**环境变量**传递而非字符串插值——含空格的路径才安全。这是从 shell 脚本生态学来的稳妥做法。

### StreamSink：边下边处理

```rust
// 边下边算哈希，无需第二遍读盘
let mut tee = TeeSink::new(HashSink::sha256(), MemorySink::default());
download.stream(url, options, &mut tee).await?;
tee.finalize().await?;
println!("{}", tee.first().hex_digest().unwrap());
```

`Download::stream()` 不落盘，五个内置 sink（File/Memory/Hash/Tee/Stdout/Gunzip）。CLI 侧：

```bash
siwi-download URL --stdout | tar xz    # 二进制安全管道
```

### 一个值得记录的技术死胡同

`flate2::read::MultiGzDecoder` **无法流式喂入**：底层输入暂时耗尽时报硬性 `UnexpectedEof` 且破坏读取状态（我们用隔离实验确认了这不是用法问题）。最终 `GunzipSink` 改为直接驱动 `flate2::Decompress` raw-deflate 模式 + 手写 RFC 1952 header/trailer 框架，支持多成员流、每成员校验 CRC32/ISIZE。

**教训：不是所有"读起来像流"的 API 都真的支持流式增量输入，用实验确认比读文档猜更可靠。**

这个版本同时补上了集成测试缺口：测试内起 mock HTTP server，断言真实下载链路的事件顺序。Windows CI 随即抓到一个真实 bug——`file_path` 用字符串拼接产生 `C:\dir/file.bin` 混合分隔符，改为 `Path::join` 修复。

---

## 2.3 — 从单文件到工作流

主题：**下载一百个文件，一半依赖另一半，怎么管？**

```rust
let mut queue = DownloadQueue::new(2);
queue.push(DownloadTask::new("model", url).output("./models"));
queue.push(DownloadTask::new("data", url).depends_on(["model"]));
let results = queue.run(&downloader, &options).await?;
```

或直接用 manifest：

```toml
# batch.toml
concurrent = 2
[[tasks]]
id = "model"
url = "https://example.com/model.bin"
output = "./models"
[[tasks]]
id = "dataset"
url = "https://example.com/dataset.tar.gz"
depends_on = ["model"]
```

```bash
siwi-download --batch batch.toml   # 失败 exit 10
```

设计要点：

- **Kahn 环检测**前置，错误信息带具体环（`y -> x -> y`）
- 失败依赖**级联跳过**——跳过的任务报告 `skipped: dependency 'x' failed`，绝不静默忽略
- 单任务失败不炸整个队列；独立任务照常并发
- `save_state`/`load_state` 持久化任务定义，中断重跑靠断点续传接续文件进度

**smoke 测试抓到的 bug**：manifest 里 `output = "./downloads"` 被静默丢弃——Rust 字段叫 `maybe_output`，serde 键名不匹配。加 `#[serde(rename = "output")]` 修复。**教训：serde 字段名与用户面向的 schema 名必须显式对齐，"碰巧同名"不是契约。**

---

## 2.4 — 生产就绪的最后一公里

主题：**跑在生产环境里，出了问题你看得见吗？**

### Prometheus 指标

```rust
let metrics = Arc::new(Metrics::new());
options.add_hook(metrics.clone());          // 就是一个普通 hook
// 之后：
println!("{}", metrics.render_prometheus());
```

```
siwi_downloads_total{status="complete"} 42
siwi_download_bytes_total 1234567890
siwi_download_resume_bytes_total 987654
siwi_download_retries_total 12
```

无锁原子计数器，`Arc` 共享跨并发下载。CLI 一键导出：

```bash
siwi-download URL --metrics-file m.prom --access-log a.jsonl
```

### 访问日志 + 优雅退出

每次下载终态一行 JSON（ts/url/size/resumed/time_used/…），可接 ELK/Loki。**Ctrl+C 现在是协作式取消**：当前 chunk 写完、flush、部分文件保留可续传，而不是粗暴 kill。

### dry-run

```bash
siwi-download URL --dry-run -j    # 只发 HEAD：多大？活着吗？要不要续传？
```

`Download::probe()` 复用与 `download()` 完全相同的 Range/条件请求头和重试逻辑——**同一套语义，只少一个 GET**。

---

## 质量账本

| 指标 | 1.0.1 → 2.4.0 |
|---|---|
| 测试 | 24 → **124**（含 mock server 集成测试） |
| clippy | 默认 → **pedantic 零告警** |
| CI | 无 → 三平台 × {stable, 1.85 MSRV} + fmt + clippy + `--all-features` |
| 模块 | 2 文件 → `download/{mod,options,report,checksum,events,sink,queue,observability,client}` |
| 发布 | 手动 → 打 tag 全自动（crates.io 手动 + cargo-dist 二进制） |

**贯穿 2.x 的三条纪律**（也是给同类项目的建议）：

1. **每个版本一个主题，先写验收标准再写代码**——避免"顺手加一个"式膨胀
2. **真机 smoke 测试是 CI 抓不到的最后一道网**——serde 键名、stdout 污染、Windows 路径都是它抓到的
3. **破坏性变更集中在大版本**——2.x 内部公开 API 只加不改，upgrade 路径丝滑

---

## 3.0 预告：Pipeline

2.2 的 `StreamSink` 是 3.0 的预演。3.0 要把架构收口为：

```
Source(HTTP/S3/File) → Filter(Gunzip/Checksum/Tee) → Sink(File/S3/Stdout)
```

标志性能力：**HTTP → S3 直传**，数据全程不落本地磁盘。`Download` 将退为兼容包装（`#[deprecated]`）。详细设计 RFC 已发布：[docs/design/30-pipeline.md](./design/30-pipeline.md)，欢迎在 Issues 里参与讨论。

## 链接

- 🏠 仓库：https://github.com/rs-videos/siwi-download
- 📦 crates.io：https://crates.io/crates/siwi-download
- 📖 API 文档：https://docs.rs/siwi-download
- 📋 完整变更：[CHANGELOG.md](../CHANGELOG.md) · 路线图：[ROADMAP.md](./ROADMAP.md)

觉得有用的话，去仓库点个 ⭐ 是对作者最大的支持。
