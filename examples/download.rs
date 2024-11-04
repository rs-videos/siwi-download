use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
use siwi_download::{
  download::{Download, DownloadOptions},
  error::AnyResult,
};
use tracing::Level;
use tracing_subscriber::FmtSubscriber;

#[tokio::main]
async fn main() -> AnyResult<()> {
  let subscriber = FmtSubscriber::builder()
    .with_max_level(Level::INFO)
    .finish();
  tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");
  let url = "https://npmmirror.com/mirrors/node/v20.18.0/node-v20.18.0.pkg";
  let mut storage_path = std::env::current_dir()?;
  storage_path.push("storage");
  let storage_path = storage_path.to_str().unwrap();
  let mut options = DownloadOptions::default();
  let mut headers = HeaderMap::new();
  headers.insert(USER_AGENT, HeaderValue::from_str("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36")?);
  options
    .set_headers(headers)
    .set_file_name("node-v20.18.0.pkg")
    .set_show_progress(true);

  let download = Download::new(storage_path);
  download.auto_create_storage_path().await?;

  let report = download.download(url, options).await?;
  println!("report {:#?}", report);
  Ok(())
}
