#[macro_use]
extern crate tracing;

use siwi_download::download::Download;
use siwi_download::download::DownloadOptions;
use siwi_download::error::AnyResult;
use tracing::Level;
use tracing_subscriber::FmtSubscriber;

#[tokio::main]
async fn main() -> AnyResult<()> {
  let subscriber = FmtSubscriber::builder()
    .with_max_level(Level::INFO)
    .finish();
  tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

  let args: Vec<String> = std::env::args().collect();
  let storage_path = std::env::current_dir()?;
  let storage_path = storage_path.to_str().unwrap_or("");

  if let Some(url) = args.get(1) {
    let mut options = DownloadOptions::default();
    options.set_show_progress(true);
    let download = Download::new(storage_path);
    let report = download.download(url, options).await?;
    info!("{:#?}", report);
    info!("report json {}", serde_json::to_string_pretty(&report)?);
  }
  Ok(())
}
