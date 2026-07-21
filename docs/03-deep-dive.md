# 拆解 siwi-download：Rust 异步下载器是怎么炼成的

> 上一篇：[《5 分钟玩转 siwi-download》](./02-getting-started.md)
> 本文假设读者对 Rust 有基本了解（async/await、所有权、trait）。

这篇文章不是使用教程，而是**源码导览**。我会带你走一遍 siwi-download 的核心实现，重点拆四个问题：

1. **断点续传**是怎么用 HTTP 协议实现的？
2. **异步下载**的完整时序是怎样的？
3. **错误处理与重试**策略是什么？
4. 为什么 2.0 版本要**移除 `Cow<'a, str>`**？

读完你会对"如何用 Rust 写一个现代异步 HTTP 工具"有完整认识。

---

## 一、项目结构：小而清晰的模块划分

siwi-download 的源码只有 ~1400 行，但模块划分非常讲究：

```
src/
├── lib.rs              crate 入口，启用 clippy::pedantic
├── main.rs             CLI（clap）
├── error.rs            AnyError / AnyResult（anyhow 别名）
├── utils.rs            URL 解析、async fs、时间戳
└── download/
    ├── mod.rs          Download 类型 + download() 核心
    ├── options.rs      DownloadOptions + builder
    ├── report.rs       DownloadReport + DownloadStatus
    └── client.rs       build_client()（提取出来的 client 构造）
```

**设计哲学**：每个文件单一职责。`download.rs` 在 1.x 是个 780 行的巨石，2.0 拆成目录后，`mod.rs` 只剩 390 行，且每个子模块都能独立测试和理解。

这种"**按类型拆模块**"（而非按函数拆）的方式是现代 Rust 项目的常见做法，便于扩展（比如未来加 `download/parallel.rs` 就能支持并发分片）。

---

## 二、断点续传：HTTP Range 协议实战

### 2.1 HTTP Range 是什么？

HTTP/1.1 的 Range 协议允许客户端只请求资源的**一部分字节**。典型流程：

```
# 客户端：我已经有 0~1023 字节，给我从 1024 开始的
GET /file.zip HTTP/1.1
Range: bytes=1024-

# 服务器：好的，剩下还有这么多
HTTP/1.1 206 Partial Content
Content-Length: 1047552
Content-Range: bytes 1024-1048575/1048576

<剩余字节流>
```

关键点：
- 客户端发 `Range: bytes=<起始>-`
- 服务器若支持，返回 `206 Partial Content` + `Content-Range`
- 若不支持，返回 `200 OK` 全量响应

### 2.2 siwi-download 怎么用 Range？

`src/download/mod.rs` 的核心代码只有几行：

```rust
// 1. 检查本地已下载的字节数
let local_size = get_file_size(file_path).await;

// 2. 构造 Range 头
let range = format!("bytes={local_size}-");
headers.insert(RANGE, HeaderValue::from_str(&range)?);

// 3. 发请求
let client = client::build_client(options.maybe_proxy.as_deref(), headers)?;
let resp = client.get(url).send().await?;
```

就这么简单。**本地文件大小 = 已下字节 = Range 起点**。下次再跑时，如果文件已存在，`get_file_size` 返回非零值，请求自动从那个位置续上。

### 2.3 三种服务器响应的处理

服务器对 Range 请求有三种典型响应，siwi-download 分别处理：

```rust
if status >= HTTP_REDIRECT_THRESHOLD {
    if status == HTTP_RANGE_NOT_SATISFIABLE {
        // 416 Range Not Satisfiable：本地文件已经下完了
        report.set_download_status(DownloadStatus::Exists);
        return Ok(report);
    }
    // 其他 4xx/5xx：失败
    report.set_download_status(DownloadStatus::Error);
    return Ok(report);
}

let total = if status == HTTP_PARTIAL_CONTENT {
    // 206 Partial Content：服务器支持 Range
    // Content-Length 是「剩余」字节，所以总大小 = 本地 + 剩余
    local_size + content_length
} else {
    // 200 OK：服务器忽略了 Range，要全量重下
    content_length
};
```

**这是一个很关键的正确性细节**：很多 naive 实现会把 `Content-Length` 直接当成总大小，但在 206 响应里 `Content-Length` 是**剩余字节数**，不是总字节数。siwi-download 2.0 在这里做了显式区分。

### 2.4 文件追加写入

下载 body 时，文件以 append 模式打开：

```rust
let mut dest = fs::OpenOptions::new()
    .create(true)
    .append(true)   // 关键：追加，不是截断
    .open(file_path)
    .await?;

while let Some(chunk) = resp.chunk().await? {
    dest.write_all(&chunk).await?;
}
```

`.append(true)` 配合 `.create(true)`：
- 文件不存在 → 创建
- 文件存在 → 在末尾追加，**不会覆盖已有部分**

这就是断点续传的"写入端"——配合 Range 请求的"读取端"，构成完整闭环。

---

## 三、异步下载时序

### 3.1 完整时序图

```
download() 调用
   │
   ├─ get_file_size(file_path).await     ──┐
   │                                        │ 全部 async
   ├─ build_client(proxy, headers)         │ 不阻塞 tokio
   │                                        │
   ├─ client.head(url).send().await  ──────┤
   │   └─ 重试循环（最多 5 次，每次 sleep 3s）
   │                                        │
   ├─ client.get(url).send().await  ───────┤
   │                                        │
   ├─ while let Some(chunk) = resp.chunk()  │
   │     dest.write_all(chunk).await  ─────┘
   │
   └─ dest.flush().await → 返回 DownloadReport
```

### 3.2 为什么所有 I/O 都是 async？

这是 siwi-download 工程上的一个亮点。看 `utils.rs`：

```rust
// ❌ 反面教材（早期版本曾这样）
pub fn is_file(dest: impl AsRef<Path>) -> AnyResult<bool> {
    let metadata = Path::new(dest.as_ref()).metadata()?;  // 同步阻塞！
    Ok(metadata.is_file())
}

// ✅ 2.0 的写法
pub async fn is_file(dest: impl AsRef<Path>) -> bool {
    match tokio::fs::metadata(dest.as_ref()).await {
        Ok(metadata) => metadata.is_file(),
        Err(_) => false,
    }
}
```

**为什么重要**：在 tokio 多线程 runtime 里，同步阻塞 I/O 会霸占 worker 线程。如果你的服务同时处理几百个下载请求，一个同步 `metadata()` 调用就会拖慢整个 runtime。

siwi-download 2.0 把 `is_file` / `is_dir` / `get_file_size` 全部 async 化，从源头杜绝了这个问题。

### 3.3 流式写入：避免内存爆炸

下载大文件时，**不能把整个 body 读到内存再写盘**。siwi-download 用 reqwest 的 streaming API：

```rust
while let Some(chunk) = resp.chunk().await? {
    dest.write_all(&chunk).await?;
}
```

`resp.chunk()` 每次返回一个 `Bytes`（通常 8KB~64KB），内存占用恒定，下载 10GB 文件也不会 OOM。

---

## 四、错误处理与重试

### 4.1 HEAD 请求的重试策略

下载前先发 HEAD 请求探测服务器（拿到 Content-Length、确认支持 Range）。但网络不稳定，HEAD 可能失败。看 `mod.rs` 的重试循环：

```rust
let mut resp = client.head(url_ref).send().await?;
let mut status = resp.status().as_u16();
let mut attempt: u32 = 0;

while !is_acceptable_status(status) && attempt < MAX_HEAD_REQUEST_RETRIES {
    attempt += 1;
    warn!(attempt, status, "HEAD request returned non-acceptable status, retrying");
    sleep(Duration::from_secs(HEAD_REQUEST_RETRY_DELAY_SECS)).await;
    resp = client.head(url_ref).send().await?;
    status = resp.status().as_u16();
}
```

`is_acceptable_status` 判断"可接受的状态码"：

```rust
fn is_acceptable_status(status: u16) -> bool {
    matches!(status, HTTP_OK | HTTP_PARTIAL_CONTENT | HTTP_RANGE_NOT_SATISFIABLE)
    // 即 200 / 206 / 416
}
```

**策略**：固定间隔（3 秒）+ 固定上限（5 次）。简单但有界，不会无限重试拖死调用方。

### 4.2 一个 1.x 版本里的 bug

这里有个值得讲的反面案例。siwi-download 1.x 的重试代码是这样：

```rust
// ❌ 1.x 的 buggy 版本
if status == HTTP_PARTIAL_CONTENT
  || status == HTTP_RANGE_NOT_SATISFIABLE
  || status >= MAX_HEAD_REQUEST_RETRIES  // ← 这里有 bug
{
    break;
}
```

`MAX_HEAD_REQUEST_RETRIES` 是重试**次数**常量（=5），却被拿来和 HTTP 状态码比较。由于 HTTP 状态码几乎都 ≥ 100，`status >= 5` 永远为真，**循环第一次迭代就 break，重试机制完全失效**。

2.0 版本把"状态判断"和"重试计数"分离成两个独立的条件，彻底修掉了这个 bug。

**教训**：常量命名要语义化。如果当时叫 `MAX_RETRY_COUNT`，可能就不会被误用作状态码阈值。

### 4.3 错误类型设计

siwi-download 用 `anyhow` 作为错误类型：

```rust
// src/error.rs
pub type AnyError = anyhow::Error;
pub type AnyResult<T> = anyhow::Result<T, AnyError>;
```

为什么不用自定义 error enum？

- **应用级 vs 库级**：作为终端工具，错误最终是给用户看的，不需要 caller 精确 match
- **错误源多样**：HTTP 错误、文件错误、URL 解析错误……用 `thiserror` 定义 union type 会很啰嗦
- **`anyhow` 的 `.context()` 足够**：需要时可以加上下文

```rust
client.get(url).send().await.context("下载请求失败")?;
```

如果是写一个被广泛依赖的**库**，应该考虑自定义 error 类型；但作为下载器这种应用级工具，anyhow 是合理的权衡。

---

## 五、HTTP client 的构造：一个被忽视的细节

2.0 把 client 构造提取到了 `download/client.rs`，看着是个小重构，其实修了一个潜在问题：

```rust
// src/download/client.rs
pub fn build_client(proxy: Option<&str>, headers: HeaderMap) -> AnyResult<reqwest::Client> {
    let builder = reqwest::Client::builder().default_headers(headers);
    let builder = match proxy {
        Some(proxy_url) => builder.proxy(reqwest::Proxy::all(proxy_url)?),
        None => builder.no_proxy(),
    };
    Ok(apply_timeouts(builder).build()?)
}

fn apply_timeouts(builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    builder
        .timeout(Duration::from_secs(60))        // 请求总超时
        .connect_timeout(Duration::from_secs(10)) // 连接超时
}
```

**1.x 版本的问题**：
1. proxy / no-proxy 两个分支各自写了一遍 timeout 配置，容易漂移
2. 完全没设 timeout，慢服务器能让进程永久挂起

**2.0 的改进**：
- 超时配置收敛到 `apply_timeouts` 一处，两条路径都走它，保证策略一致
- 显式设置 60s 请求超时 + 10s 连接超时

这是工程上的"小处不可随便"。提取一个 helper 看似简单，但避免了"复制粘贴导致配置不一致"这个经典陷阱。

---

## 六、为什么 2.0 移除了 `Cow<'a, str>`？

这是 2.0 最大的破坏性变更，值得专门讲讲。

### 6.1 1.x 的 API

```rust
// 1.x
pub struct DownloadOptions<'a> {
    pub maybe_file_name: Option<Cow<'a, str>>,
    pub maybe_proxy: Option<Cow<'a, str>>,
}

impl<'a> DownloadOptions<'a> {
    pub fn set_file_name<S: Into<Cow<'a, str>>>(&mut self, file_name: S) -> &mut Self {
        self.maybe_file_name = Some(file_name.into());
        self
    }
}
```

`Cow<'a, str>` 看起来很美：调用方传 `&str` 时零拷贝，传 `String` 时自动 owned。但实际使用中：

```rust
// 调用点几乎全是这种
options.set_file_name("test.txt");              // &'static str
options.set_file_name(format!("{}.zip", name)); // String
```

`&'static str` 的场景，`Cow::Borrowed` 确实省了一次 allocation。但代价是：

### 6.2 代价 1：生命周期污染

`'a` 出现在 struct 定义里，会污染所有持有它的类型：

```rust
pub struct Download<'a> {
    pub storage_path: Cow<'a, str>,
}

pub struct DownloadReport<'a> {
    pub url: Cow<'a, str>,
    pub file_name: Cow<'a, str>,
    // ... 十几个字段
}

// 用户写代码时
let downloader: Download<'_> = Download::new(...);  // 看着就累
```

### 6.3 代价 2：实际收益约等于零

siwi-download 的使用模式是「配置一次 → 下载 → 拿 report」，配置对象生命周期很短。省下来的那一次 allocation 根本不是瓶颈。

**性能优化要讲 ROI**。`Cow<'a, str>` 在解析器、序列化器这种"高频调用 + 真有零拷贝场景"的地方才有意义；在配置对象上是过度设计。

### 6.4 2.0 的简化

```rust
// 2.0
pub struct DownloadOptions {
    pub maybe_file_name: Option<String>,
}

impl DownloadOptions {
    pub fn set_file_name<S: Into<String>>(&mut self, file_name: S) -> &mut Self {
        self.maybe_file_name = Some(file_name.into());
        self
    }
}
```

- struct 没有生命周期参数，使用处清爽
- `impl Into<String>` 依然接收 `&str` / `String` / `&String`，调用方便利性不变
- 内部一律 owned，避免「Borrowed 的 Cow 被 lifetime 卡住」的尴尬

**教训**：不要为了"看起来高级"而引入复杂度。`Cow` 是工具，不是装饰品。

---

## 七、Builder Pattern 的标准实现

siwi-download 的 `DownloadOptions` 是个教科书级的 builder：

```rust
#[derive(Debug, Default)]
pub struct DownloadOptions {
    pub maybe_file_name: Option<String>,
    pub maybe_proxy: Option<String>,
    pub maybe_headers: Option<HeaderMap>,
    pub show_progress: bool,
}

impl DownloadOptions {
    pub fn set_file_name<S: Into<String>>(&mut self, file_name: S) -> &mut Self {
        self.maybe_file_name = Some(file_name.into());
        self
    }
    // ... 其他 setter 同构
}
```

几个细节值得学习：

1. **`&mut self` 而非 `self`**：返回 `&mut Self` 允许链式调用的同时，配置对象还能继续使用（`&mut self` 借用结束后释放）
2. **`impl Into<String>` 泛型**：接收多种字符串类型，调用方零转换
3. **`#[derive(Default)]`**：`DownloadOptions::default()` 直接可用，新字段加上后老代码不破
4. **`Option<T>` 字段**：所有可选配置都是 `Option`，区分「未设置」和「设置为默认值」

这是 Rust 社区主流的"mutable builder"风格，比 consuming builder（`fn foo(self)`）灵活。

---

## 八、用 indicatif 做进度条

进度条是个看似简单但坑很多的特性。siwi-download 的实现：

```rust
fn build_progress_bar(total: u64, partial: bool, local_size: u64) -> ProgressBar {
    let pb = ProgressBar::new(total);
    pb.set_style(
        ProgressStyle::with_template(
            "{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({eta})"
        )
        .unwrap()
        .with_key("eta", |state: &ProgressState, w: &mut dyn Write| {
            let eta = state.eta().as_secs_f64();
            if eta.is_finite() {
                let _ = write!(w, "{eta:.1}s");
            }
        })
        .progress_chars("#>-"),
    );
    // 续传时把进度条初始位置设到已下载处
    if partial && local_size > 0 {
        pb.set_position(local_size);
    }
    pb
}
```

**两个值得注意的点**：

1. **自定义 ETA formatter**：默认的 `{eta}` 只显示整数秒，自定义闭包显示一位小数，更精确
2. **续传场景的初始位置**：`pb.set_position(local_size)` 让进度条从"已下载位置"开始，而不是从 0 跳跃。这种细节决定用户体验

另外，2.0 把进度条的构造包成了 `Option<ProgressBar>`：

```rust
let pb = if options.show_progress {
    Some(build_progress_bar(...))
} else {
    None
};

while let Some(chunk) = resp.chunk().await? {
    dest.write_all(&chunk).await?;
    if let Some(pb) = pb.as_ref() {
        pb.inc(chunk.len() as u64);
    }
}
```

不显示进度条时根本不创建对象，零开销。

---

## 九、CI / 工程化的讲究

最后讲讲工程化。siwi-download 2.0 的 CI 矩阵：

```yaml
# .github/workflows/ci.yml
test:
  strategy:
    matrix:
      os: [ubuntu-latest, macos-latest, windows-latest]
      rust: [stable, "1.85"]   # MSRV
```

三平台 × 两版本 = 6 个 job，加上独立的 fmt / clippy / msrv-verify。

几个有意思的配置：

```yaml
env:
  RUSTFLAGS: "-D warnings"   # 把 warning 当 error
```

```rust
// src/lib.rs
#![warn(clippy::pedantic)]   # 启用 pedantic 风格检查
```

**`-D warnings` + `clippy::pedantic`** 的组合保证了代码质量不会随时间腐化。任何 PR 只要有 warning 就过不了 CI。

MSRV 验证也很重要：

```rust
# Cargo.toml
rust-version = "1.85"
```

```yaml
# CI 里单独跑 1.85 工具链
- uses: dtolnay/rust-toolchain@stable
  with:
    toolchain: "1.85"
```

这保证 siwi-download 不会意外用了 1.86+ 才有的特性，让低版本 Rust 用户也能用。

---

## 十、总结：能从 siwi-download 学到什么

| 维度 | 学到的东西 |
|---|---|
| **协议** | HTTP Range 协议的实战应用，206 vs 200 vs 416 的区分 |
| **async** | tokio 全栈 async 的设计模式，避免阻塞 I/O |
| **API 设计** | 何时该用 / 不该用 `Cow<'a, str>`，mutable builder 的标准写法 |
| **错误处理** | anyhow 的适用场景，重试循环的有界性 |
| **工程化** | 模块拆分、CI 矩阵、MSRV 声明、pedantic clippy |
| **代码品味** | 提取 helper 消除重复，`Option<T>` 的零开销设计 |

这个项目不大（~1400 行），但每一处都体现了"小项目也可以很讲究"的工程态度。如果你正在找一个**可以一口气读完**的现代 Rust 项目来学习，siwi-download 是个不错的选择。

---

## 延伸阅读

- [HTTP Range Requests (MDN)](https://developer.mozilla.org/en-US/docs/Web/HTTP/Range_requests)
- [tokio Tutorial](https://tokio.rs/tokio/tutorial)
- [reqwest 文档](https://docs.rs/reqwest)
- [clippy::pedantic lint 列表](https://rust-lang.github.io/rust-clippy/master/index.html)

## 项目链接

- 🏠 仓库：https://github.com/rs-videos/siwi-download
- 📦 crates.io：https://crates.io/crates/siwi-download
- 📖 API 文档：https://docs.rs/siwi-download
- 📋 更新日志：[CHANGELOG.md](../CHANGELOG.md)

欢迎 star ⭐、提 issue、贡献 PR。如果你从这篇文章学到了东西，转发给身边的 Rust 学习者是对作者最大的支持。
