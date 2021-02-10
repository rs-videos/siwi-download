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
    let report = Download::new(storage_path).download(url, options).await?;
    println!("{:?}", report);
  }
  Ok(())
}
