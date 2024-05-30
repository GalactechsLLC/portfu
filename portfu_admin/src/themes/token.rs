use crate::stores::DataStoreEntry;

#[derive(Default, Clone, Eq, PartialEq)]
pub struct Token {
    pub key: String,
    pub value: String,
}
impl Token {
    pub const TITLE: &'static str = "<!--{title-token}-->";
    pub const PAGE: &'static str = "<!--{page-token}-->";
    pub const META: &'static str = "<!--{meta-token}-->";
    pub const CSS_PATH: &'static str = "<!--{css-path-token}-->";
    pub const JS_PATH: &'static str = "<!--{js-path-token}-->";
}
impl DataStoreEntry<String> for Token {
    fn key_name() -> &'static str {
        "key"
    }
    fn key_value(&self) -> String {
        self.key.clone()
    }

    fn parameters() -> &'static [&'static str] {
        &["key", "value"]
    }

    fn matches(&self, name: &str, other: &str) -> bool {
        match name {
            "key" => self.key == other,
            "value" => self.value == other,
            _ => false,
        }
    }
}