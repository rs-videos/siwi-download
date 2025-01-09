pub mod download;
pub mod error;
pub mod utils;

#[cfg(test)]
mod tests {
  use super::{
    error::AnyResult,
    utils::{gen_file_name, get_file_name_from_url},
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
}
