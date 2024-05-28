use portfu::prelude::uuid::Uuid;
use std::collections::HashMap;
use struct_field_names_as_array::FieldNamesAsSlice;
use crate::stores::DataStoreEntry;


#[derive(Default, Clone, FieldNamesAsSlice)]
pub struct PageMetadata {
    pub title: String,
    pub tags: HashMap<String, String>,
}

#[derive(Default, Clone, FieldNamesAsSlice)]
pub struct Page {
    pub id: isize,
    pub uuid: Uuid,
    pub title: Option<String>,
    pub path: String,
    pub metadata: Option<PageMetadata>,
    pub html: String,
    pub css: String,
    pub js: String,
}
impl DataStoreEntry<isize> for Page {
    fn key_name() -> &'static str {
        "id"
    }
    fn key_value(&self) -> isize {
        self.id
    }

    fn parameters() -> &'static [&'static str] {
        Self::FIELD_NAMES_AS_SLICE
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
