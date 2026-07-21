# 5 分钟玩转 siwi-download：CLI + Rust 库上手指南

> 上一篇：[《siwi-download：一个用 Rust 写的轻量下载器》](./01-introduction.md)

这篇是纯上手教程。读完你能：

- 用 CLI 完成日常所有下载任务
- 把它当 Rust 库嵌进自己的程序
- 处理代理、续传、脚本化等典型场景

---

## 一、CLI 篇

### 1.1 安装

```bash
cargo install siwi-download
# 验证
siwi-download --version
# siwi-download 2.0.0
```

### 1.2 命令全景

```
siwi-download [OPTIONS] [url]

Arguments:
  [url]  URL to download (positional, or use --url)

Options:
  -u, --url <url_flag>       URL（位置参数的替代写法）
  -o, --output <output>      输出目录 [默认: .]
  -f, --filename <filename>  自定义文件名
  -P, --progress             显示进度条
  -p, --proxy <proxy>        HTTP 代理
  -v, --verbose              详细日志
  -j, --json                 JSON 格式输出
  -h, --help                 帮助
  -V, --version              版本
```

URL 既可以放在第一个位置参数，也可以用 `-u/--url` 显式指定。两种写法等价：

```bash
siwi-download https://example.com/file.zip
siwi-download -u https://example.com/file.zip
```

**贴心细节**：什么都不传时，CLI 会直接打印完整帮助（和 `git`/`cargo` 一样），不会让你对着一个 `Error: URL is required` 发呆。

### 1.3 五个典型场景

#### 场景 1：最朴素的下载

```bash
siwi-download https://example.com/file.zip
```

文件会被下载到当前目录，文件名从 URL 自动提取。

#### 场景 2：要进度条

```bash
siwi-download https://example.com/big.iso -P
```

渲染效果（indicatif 实现）：

```
⠁ [00:00:05] [####>----------] 50MiB/120MiB (8.2s)
```

包含已下载字节、总字节、预估剩余时间。

#### 场景 3：指定目录和文件名

```bash
siwi-download https://example.com/latest/release.tar.gz \
  -o ~/Downloads \
  -f myapp-v1.0.tar.gz
```

- `-o` 指定输出目录（不存在会自动创建）
- `-f` 覆盖文件名（默认从 URL 末段提取）

#### 场景 4：通过代理下载

```bash
siwi-download https://example.com/file.zip -p http://127.0.0.1:7890
```

支持 HTTP/HTTPS 代理，对 clash / v2ray / 公司网关等场景友好。

#### 场景 5：脚本化，要 JSON 输出

```bash
siwi-download https://example.com/file.zip -j
```

输出标准 JSON，方便 `jq` 解析：

```json
{
  "url": "https://example.com/file.zip",
  "file_name": "file.zip",
  "origin_file_name": "file.zip",
  "storage_path": ".",
  "file_path": "./file.zip",
  "file_size": 1048576,
  "range_from": 0,
  "download_start_at": "2026-07-21T10:30:00Z",
  "download_end_at": "2026-07-21T10:30:05Z",
  "download_status": "Complete",
  "head_status": 200,
  "resp_status": 206,
  "time_used": 5,
  "msg": null
}
```

配合 `jq` 提取关键字段：

```bash
siwi-download https://example.com/file.zip -j | jq '.file_path,.time_used'
```

### 1.4 续传：核心卖点

这是 siwi-download 最值得用的特性。**只要再跑一次同样命令**：

```bash
# 第一次：下到一半 Ctrl+C
siwi-download https://example.com/big.iso -o ./dl -P

# 第二次：自动从断点继续
siwi-download https://example.com/big.iso -o ./dl -P
```

原理：CLI 会先检查本地文件大小，发 `Range: bytes=<已下大小>-` 请求给服务器，只下载剩余部分。注意几个边界：

- ✅ **服务器必须支持 Range**（大多数 CDN、对象存储都支持）
- ⚠️ 如果服务器返回 `200 OK`（忽略 Range），会从头重传
- ✅ 如果本地已是完整文件，服务器返回 `416 Range Not Satisfiable`，CLI 会识别并直接报告 "file exists"

### 1.5 调试小技巧

加 `-v` 看详细日志（`tracing` 输出）：

```bash
siwi-download https://example.com/file.zip -v
```

会打印 HEAD 请求重试、Content-Length、storage 目录创建等内部事件，排查问题很有用。

---

## 二、Rust 库篇

### 2.1 引入依赖

```toml
# Cargo.toml
[dependencies]
siwi-download = "2"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

### 2.2 最小示例

```rust
use siwi_download::download::{Download, DownloadOptions};

#[tokio::main]
async fn main() -> siwi_download::error::AnyResult<()> {
    // 1. 指定存储目录（不存在会自动创建）
    let downloader = Download::new("./downloads");
    downloader.auto_create_storage_path().await?;

    // 2. 配置选项（builder pattern）
    let mut options = DownloadOptions::default();
    options
        .set_file_name("custom-name.zip")
        .set_show_progress(true);

    // 3. 执行下载
    let report = downloader
        .download("https://example.com/file.zip", options)
        .await?;

    println!("下载完成: {:#?}", report);
    Ok(())
}
```

### 2.3 API 速查

#### `Download` —— 下载器实例

```rust
// 创建：传入存储目录
let downloader = Download::new("./downloads");

// 自动创建目录（幂等）
downloader.auto_create_storage_path().await?;

// 下载（&self，可复用）
let report = downloader.download(url, options).await?;
```

**关键设计**：`download` 接收 `&self`，所以你可以复用同一个实例下多个文件：

```rust
let downloader = Download::new("./downloads");
for url in urls {
    downloader.download(url, DownloadOptions::default()).await?;
}
```

#### `DownloadOptions` —— 配置项（builder）

```rust
let mut options = DownloadOptions::default();
options
    .set_file_name("photo.jpg")              // 自定义文件名
    .set_proxy("http://127.0.0.1:7890")      // 代理
    .set_show_progress(true);                // 进度条

// 自定义 HTTP headers（比如加 User-Agent）
use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
let mut headers = HeaderMap::new();
headers.insert(USER_AGENT, HeaderValue::from_str("my-app/1.0")?);
options.set_headers(headers);
```

#### `DownloadReport` —— 下载结果

下载完成后返回，包含完整元数据：

```rust
pub struct DownloadReport {
    pub url: String,
    pub file_name: String,
    pub origin_file_name: String,
    pub storage_path: String,
    pub file_path: String,
    pub file_size: Option<u64>,
    pub range_from: Option<u64>,
    pub download_status: Option<DownloadStatus>,  // Create/Append/Complete/Exists/Error
    pub time_used: Option<i64>,                   // 秒
    pub msg: Option<String>,
    // ... 完整字段见 API 文档
}
```

`DownloadStatus` 枚举反映下载生命周期：

```rust
pub enum DownloadStatus {
    Create,   // 新文件，从 0 开始下
    Append,   // 续传，追加到已有文件
    Complete, // 完成
    Exists,   // 已存在（416）
    Error,    // 失败
}
```

### 2.4 错误处理

库统一用 `AnyResult<T>`（`anyhow::Result` 别名），配合 `?` 传播：

```rust
use siwi_download::error::AnyResult;

async fn fetch(url: &str) -> AnyResult<()> {
    let downloader = Download::new("./downloads");
    downloader.download(url, DownloadOptions::default()).await?;
    Ok(())
}
```

需要更细粒度的错误信息？检查 `DownloadReport.download_status`：

```rust
let report = downloader.download(url, options).await?;
match report.download_status {
    Some(DownloadStatus::Complete) => println!("✅ 成功"),
    Some(DownloadStatus::Exists)   => println!("ℹ️ 文件已存在"),
    Some(DownloadStatus::Error)    => {
        eprintln!("❌ 下载失败: {:?}", report.msg);
        std::process::exit(1);
    }
    _ => {}
}
```

### 2.5 把报告发送到远端

`DownloadReport::report()` 内置了一个把结果 POST 到指定 URL 的方法（适合做下载埋点）：

```rust
use reqwest::header::HeaderMap;

let mut headers = HeaderMap::new();
// 加上鉴权头等
report.report("https://your-api.example.com/downloads", headers).await?;
```

它通过 serde 把自身序列化成 JSON 发送，字段会随 struct 定义自动同步，不会漂移。

---

## 三、实战配方

### 配方 1：批量下载脚本

```bash
#!/bin/bash
# batch-download.sh
set -e

URLS=(
  "https://example.com/file1.zip"
  "https://example.com/file2.zip"
  "https://example.com/file3.zip"
)

for url in "${URLS[@]}"; do
  echo ">>> 下载: $url"
  siwi-download "$url" -o ./downloads -P -j | jq '{file: .file_name, status: .download_status}'
done
```

### 配方 2：Rust 中下载列表 + 错误隔离

```rust
use siwi_download::download::{Download, DownloadOptions, DownloadStatus};

#[tokio::main]
async fn main() {
    let downloader = Download::new("./downloads");
    downloader.auto_create_storage_path().await.unwrap();

    let urls = vec![
        "https://example.com/a.zip",
        "https://example.com/b.zip",
        "https://example.com/c.zip",
    ];

    // 并发下载所有文件（一个失败不影响其他）
    let futures: Vec<_> = urls
        .into_iter()
        .map(|url| {
            let dl = &downloader;
            async move {
                let report = dl.download(url, DownloadOptions::default()).await?;
                println!("{} -> {:?}", url, report.download_status);
                Ok::<_, siwi_download::error::AnyError>(())
            }
        })
        .collect();

    futures::future::join_all(futures).await;
}
```

> 上例需要额外引入 `futures` crate。

### 配方 3：Dockerfile 里可靠下载

```dockerfile
FROM rust:1.85 AS builder
RUN cargo install siwi-download

FROM debian:bookworm-slim
COPY --from=builder /usr/local/cargo/bin/siwi-download /usr/local/bin/

# 失败会自动重试，支持续传
RUN siwi-download https://example.com/large-model.bin \
    -o /models -f model.bin -P
```

### 配方 4：CI 里抓数据集

```yaml
# .github/workflows/train.yml
- name: Download dataset
  run: |
    siwi-download https://example.com/dataset.tar.gz \
      -o ./data -f dataset.tar.gz -P -j
```

CI 失败重跑时，因为断点续传，不会浪费时间重下整个文件。

---

## 四、常见问题

### Q1：下载到一半网络断了怎么办？

直接重跑原命令。siwi-download 会检查本地文件大小，自动发 Range 请求续传。

### Q2：服务器不支持续传？

你会看到它每次都从头下载。这是服务器没实现 HTTP Range（返回 `200 OK` 而不是 `206 Partial Content`）。siwi-download 在这种情况下会优雅降级为全量下载。

### Q3：能同时下多个文件吗？

CLI 本身一次只下一个。在 Rust 库里可以自己 `tokio::spawn` 并发，参考「配方 2」。

### Q4：支持 SOCKS5 代理吗？

目前只支持 HTTP/HTTPS 代理（`reqwest::Proxy::all`）。SOCKS5 需要启用 reqwest 的 `socks` feature，欢迎提 PR。

### Q5：如何报告 bug？

到 [GitHub Issues](https://github.com/rs-videos/siwi-download/issues) 提交，附上：
- `siwi-download --version`
- 完整命令
- `-v` 详细日志输出

---

## 小结

CLI 层面，siwi-download 覆盖了日常下载的绝大多数场景；库层面，它的 async API 设计干净、文档完整、测试齐全。

如果读完想看看「断点续传是怎么实现的」「tokio 异步下载器内部长什么样」，继续看下一篇技术深度文章。

---

**下一篇**：[《拆解 siwi-download：Rust 异步下载器是怎么炼成的》](./03-deep-dive.md)
