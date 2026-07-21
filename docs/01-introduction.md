# siwi-download：一个用 Rust 写的轻量下载器

> 仓库地址：https://github.com/rs-videos/siwi-download
> 当前版本：v2.0.0 · 许可证：MIT · MSRV：1.85

## 一句话介绍

**siwi-download** 是一个用 Rust 编写的命令行 + 库二合一文件下载器，核心卖点是**断点续传**：下载中断后，再次执行同一命令会从断开的位置继续，而不是从头开始。

如果你经常下载大文件（系统镜像、数据集、视频素材），或者需要在脚本里做可靠的 HTTP 下载，它可能会成为你工具链里的一把瑞士军刀。

## 为什么又造一个轮子？

`wget` / `curl` 当然能用，但它们有各自的槽点：

- **`wget`**：macOS 不再自带；GNU 风格参数冗长；在容器里装它要拉一堆依赖
- **`curl`**：自身不做断点续传（`-C -` 只是发 Range 头，不保证状态一致）
- **`aria2`**：功能强，但是 C++ 写的庞然大物，单二进制就有 10MB+
- **Python 脚本**：每次起进程慢，错误处理全靠自己

siwi-download 想做的是一个**够用、好装、可嵌入**的小工具：

- **单一二进制**：编译出来就一个可执行文件，无运行时依赖
- **跨平台**：macOS（含 ARM）/ Linux / Windows 原生支持
- **同时是 CLI 和 Rust 库**：既能当命令用，也能 `cargo add` 嵌进你自己的项目
- **现代异步**：基于 tokio + reqwest，能扛高并发场景

## 核心特性一览

| 特性 | 说明 |
|---|---|
| 🔄 **断点续传** | 基于 HTTP `Range` 头，中断后从本地已下载字节数继续 |
| 🚀 **异步 I/O** | tokio 多线程运行时，文件读写全 async |
| 📊 **进度条** | `indicatif` 渲染，含速度、ETA、字节数 |
| 🌐 **代理支持** | `--proxy` 一键走 HTTP/HTTPS 代理 |
| ⏱️ **超时保护** | 请求 60s、连接 10s，慢服务器不会挂死 |
| 📄 **JSON 报告** | `--json` 输出结构化下载结果，方便脚本解析 |
| 🔧 **可作库用** | `Download::download(&self, url, options)` 干净的 async API |
| 🎯 **无外部依赖** | rustls 实现 TLS，不依赖系统 OpenSSL |

## 安装

### 方式一：cargo（最简单）

```bash
cargo install siwi-download
```

### 方式二：从源码编译

```bash
git clone https://github.com/rs-videos/siwi-download.git
cd siwi-download
cargo build --release
# 二进制在 target/release/siwi-download
```

### 方式三：预编译二进制

到 [Releases 页面](https://github.com/rs-videos/siwi-download/releases) 下载对应平台的压缩包，或者用官方安装脚本：

**macOS / Linux：**
```bash
curl -LsSf https://github.com/rs-videos/siwi-download/releases/latest/download/siwi-download-installer.sh | sh
```

**Windows（PowerShell）：**
```powershell
irm https://github.com/rs-videos/siwi-download/releases/latest/download/siwi-download-installer.ps1 | iex
```

## 30 秒上手

```bash
# 最朴素的下载
siwi-download https://example.com/big-file.iso

# 带进度条 + 自定义目录 + 自定义文件名
siwi-download https://example.com/big-file.iso \
  -o ./downloads \
  -f ubuntu-24.04.iso \
  -P

# 下载中断了？再跑一次同样的命令，自动续传
siwi-download https://example.com/big-file.iso -o ./downloads -f ubuntu-24.04.iso -P
```

跑完会输出一份下载报告，也可以用 `-j` 拿到 JSON：

```bash
siwi-download https://example.com/file.zip -j
```

```json
{
  "url": "https://example.com/file.zip",
  "file_name": "file.zip",
  "storage_path": "/downloads",
  "file_path": "/downloads/file.zip",
  "file_size": 1048576,
  "range_from": 0,
  "download_status": "Complete",
  "time_used": 5
}
```

## 当作 Rust 库用

siwi-download 也是一个 crate，可以直接 `cargo add siwi-download`：

```rust
use siwi_download::download::{Download, DownloadOptions};

#[tokio::main]
async fn main() -> siwi_download::error::AnyResult<()> {
    let download = Download::new("./downloads");
    download.auto_create_storage_path().await?;

    let mut options = DownloadOptions::default();
    options.set_show_progress(true);

    let report = download
        .download("https://example.com/file.zip", options)
        .await?;

    println!("下载完成: {:?}", report);
    Ok(())
}
```

API 设计上有几个值得一提的点：

- `Download::download` 接收 `&self`，**一个实例可以复用**，连续下多个文件不用重建
- `DownloadOptions` 用 **builder pattern**，链式调用配置
- 所有 I/O 都是 async，**不会阻塞 tokio runtime**
- 错误统一用 `AnyResult<T>`（anyhow 别名），`?` 一路向上抛

## 工程质量

这个项目虽然小，但工程实践上不将就：

- ✅ **模块化结构**：`download/` 目录拆成 `options` / `report` / `client` / `mod`，每个文件职责单一
- ✅ **clippy::pedantic 全开**，零告警
- ✅ **MSRV 明确声明**：`rust-version = "1.85"`，CI 在 stable 和 1.85 两档验证
- ✅ **三平台 CI 矩阵**：ubuntu / macos / windows × stable / 1.85
- ✅ **完整的 CHANGELOG**（Keep a Changelog 格式，含历史版本）
- ✅ **单元测试覆盖**核心工具函数与 API 契约
- ✅ **自动发布**：打 tag 触发 cargo-dist，自动产出跨平台二进制 + 安装脚本

## 适合谁用？

- **运维 / SRE**：在 Dockerfile / CI 里下大文件，要可重试、可续传
- **数据工程师**：下数据集、模型文件，希望脚本可靠、能拿到结构化结果
- **Rust 学习者**：想读一份**小而完整**的现代 Rust 项目源码（async、错误处理、builder、clap、serde 全有）
- **开源爱好者**：想找一个友好的 Rust 项目贡献 PR

## 不适合什么场景？

诚实地说一下边界：

- ❌ **多线程分片下载**：目前不支持把一个文件切成多段并发下（aria2 的 `-x 16` 那种）。如果你极度在意单文件吞吐，aria2 仍是首选
- ❌ **BT / 磁力链**：只支持 HTTP(S)
- ❌ **FTP / SFTP**：同上，只做 HTTP

它的定位是 **HTTP(S) 单连接 + 断点续传**的轻量方案，不追求取代 aria2。

## 项目链接

- 🏠 仓库：https://github.com/rs-videos/siwi-download
- 📦 crates.io：https://crates.io/crates/siwi-download
- 📖 API 文档：https://docs.rs/siwi-download
- 📋 更新日志：[CHANGELOG.md](../CHANGELOG.md)

如果觉得有用，去仓库点个 ⭐ 是对作者最大的鼓励。Issue、PR 都欢迎。

---

**下一篇**：[《5 分钟玩转 siwi-download：CLI + Rust 库上手指南》](./02-getting-started.md)
