pub mod download;
pub mod error;
pub mod utils;

#[cfg(test)]
mod tests {
  use super::download::{gen_file_name, get_file_name_from_url, get_file_size};
  use super::error::AnyResult;
  #[test]
  fn it_works() {
    assert_eq!(2 + 2, 4);
  }
  #[test]
  fn do_get_file_name_from_url() ->AnyResult<()>{
    let url = "https://cdn.npm.taobao.org/dist/node/v14.15.4/node-v14.15.4.pkg";
    let file_name = get_file_name_from_url(url)?;
    assert_eq!("node-v14.15.4.pkg".to_owned(), file_name);
    Ok(())
  }
  #[test]
  fn do_gen_file_name() ->AnyResult<()>{
    let url = "https://cdn.npm.taobao.org/dist/node/v14.15.4/node-v14.15.4.pkg";
    let file_name = gen_file_name(url)?;
    assert_ne!("node-v14.15.4.pkg".to_owned(), file_name);
    Ok(())
  }
  #[test]
  fn do_get_file_size() ->AnyResult<()>{
    let file_path = std::env::current_dir()?;
    let file_path = format!("{}/src/lib.rs", file_path.to_str().unwrap());
    let file_name = get_file_size(file_path.as_str())?;
    assert_eq!(1032, file_name);
    Ok(())
  }
}
