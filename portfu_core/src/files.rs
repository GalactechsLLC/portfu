pub mod dynamic;
pub mod loader;
pub mod r#static;

pub enum EditResult {
    NotEditable,
    Success(Vec<u8>),
    Failed(String),
}
