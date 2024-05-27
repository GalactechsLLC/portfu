use crate::DataStoreEntry;
use portfu::prelude::uuid::Uuid;
use std::collections::HashMap;
use struct_field_names_as_array::FieldNamesAsSlice;

#[derive(Default, Clone, FieldNamesAsSlice)]
pub struct Template {
    pub id: isize,
    pub path: String,
    pub uuid: Uuid,
    pub title: String,
    pub tags: HashMap<String, String>,
    pub html: String,
    pub css: String,
    pub js: String,
}
impl DataStoreEntry for Template {
    fn key(&self) -> String {
        self.id.to_string()
    }

    fn parameters() -> &'static [&'static str] {
        &["id", "uuid"]
    }

    fn matches(&self, name: &str, other: &str) -> bool {
        if !Self::FIELD_NAMES_AS_SLICE.contains(&name) {
            return false;
        }
        match name {
            "id" => self.id.to_string() == other,
            "uuid" => self.uuid.to_string() == other,
            _ => false,
        }
    }
}
