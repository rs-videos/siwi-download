use siwi_download::{download::Download, error::AnyResult};
use siwi_download::download::DownloadReport;
use siwi_download::download::DownloadOptions;
#[tokio::main]
async fn main() ->AnyResult<()>{
  let url = "https://cdn.npm.taobao.org/dist/node/v14.15.4/node-v14.15.4.pkg";
  let storage_path = "/Volumes/ssd/volumes/code/rs-videos/siwi-download/storage";
  let mut options = DownloadOptions::default();
  options.set_file_name("hello_world.pkg");
  let report:DownloadReport = Download::new(url, storage_path).download(options).await?;

  println!("report {:#?}", report);
  Ok(())
}