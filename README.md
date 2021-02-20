# siwi-download

> download file

- 支持断点继续下载

```rust
use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
use siwi_download::{
  download::{Download, DownloadOptions},
  error::AnyResult,
};
#[tokio::main]
async fn main() -> AnyResult<()> {
  let url = "https://cdn.npm.taobao.org/dist/node/v14.15.4/node-v14.15.4.pkg";
  let mut storage_path = std::env::current_dir()?;
  storage_path.push("storage");
  let storage_path = storage_path.to_str().unwrap();
  let mut options = DownloadOptions::default();
  let mut headers = HeaderMap::new();
  headers.insert(USER_AGENT, HeaderValue::from_str("Mozilla/5.0 (Macintosh; Intel Mac OS X 11_2_0) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/88.0.4324.150 Safari/537.36")?);
  options
    .set_headers(headers)
    .set_file_name("hello_world1.pkg")
    .set_show_progress(true);

  let download = Download::new(storage_path);
  download.auto_create_storage_path().await?;
  
  let report = download.download(url, options)
    .await?;
  println!("report {:#?}", report);
  Ok(())
}


```

- Write a CLI tool

```rust
use siwi_download::download::Download;
use siwi_download::download::DownloadOptions;
use siwi_download::error::AnyResult;
#[tokio::main]
async fn main() -> AnyResult<()> {
  let args: Vec<String> = std::env::args().collect();
  let storage_path = std::env::current_dir()?;
  let storage_path = storage_path.to_str().unwrap_or("");

  if let Some(url) = args.get(1) {
    let mut options = DownloadOptions::default();
    options.set_show_progress(true);
    let download = Download::new(storage_path);
    let report = download.download(url, options).await?;
    println!("{:?}", report);
  }
  Ok(())
}


```
