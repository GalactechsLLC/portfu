pub enum EditResult {
    NotEditable,
    Success(Vec<u8>),
    Failed(String),
}
