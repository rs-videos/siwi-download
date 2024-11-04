pub mod download;
pub mod error;
pub mod utils;

#[cfg(test)]
mod tests {
  use super::{
    error::AnyResult,
    utils::{gen_file_name, get_file_name_from_url, get_file_size},
  };
  #[test]
  fn it_works() {
    assert_eq!(2 + 2, 4);
  }
  #[test]
  fn do_get_file_name_from_url() -> AnyResult<()> {
    let url = "https://nodejs.org/dist/v22.11.0/node-v22.11.0.pkg";
    let file_name = get_file_name_from_url(url)?;
    assert_eq!("node-v22.11.0.pkg".to_owned(), file_name);
    Ok(())
  }
  #[test]
  fn do_gen_file_name() -> AnyResult<()> {
    let url = "https://nodejs.org/dist/v22.11.0/node-v22.11.0.pkg";
    let file_name = gen_file_name(url)?;
    assert_ne!("node-v22.11.0.pkg".to_owned(), file_name);
    Ok(())
  }
  #[test]
  fn do_get_file_size() -> AnyResult<()> {
    let file_path = std::env::current_dir()?;
    let file_path = format!("{}/src/lib.rs", file_path.to_str().unwrap());
    let file_name = get_file_size(file_path.as_str())?;
    assert_eq!(1037, file_name);
    Ok(())
  }
}
