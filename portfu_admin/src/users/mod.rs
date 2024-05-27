pub mod manager;

use crate::DataStoreEntry;
use portfu::prelude::uuid::Uuid;
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use struct_field_names_as_array::FieldNamesAsSlice;
use time::OffsetDateTime;

#[derive(FieldNamesAsSlice, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: isize,
    pub uuid: Uuid,
    pub username: String,
    pub email: String,
    pub role: UserRole,
    pub metadata: UserMetaData,
    pub created: OffsetDateTime,
    pub updated: OffsetDateTime,
}
impl Default for User {
    fn default() -> Self {
        Self {
            id: -1,
            uuid: Default::default(),
            username: Default::default(),
            email: Default::default(),
            role: Default::default(),
            metadata: Default::default(),
            created: OffsetDateTime::now_utc(),
            updated: OffsetDateTime::now_utc(),
        }
    }
}
impl DataStoreEntry for User {
    fn key(&self) -> String {
        self.id.to_string()
    }

    fn parameters() -> &'static [&'static str] {
        User::FIELD_NAMES_AS_SLICE
    }

    fn matches(&self, name: &str, other: &str) -> bool {
        if !Self::FIELD_NAMES_AS_SLICE.contains(&name)
            && !UserMetaData::FIELD_NAMES_AS_SLICE.contains(&name)
        {
            return false;
        }
        match name {
            "id" => self.id.to_string() == other,
            "uuid" => self.uuid.to_string() == other,
            "username" => self.username == other,
            "email" => self.email == other,
            "role" => self.role.to_string().to_ascii_lowercase() == other.to_ascii_lowercase(),
            "first_name" => self.metadata.first_name == other,
            "last_name" => self.metadata.last_name == other,
            "home_phone" => self.metadata.home_phone == other,
            "work_phone" => self.metadata.work_phone == other,
            "cell_phone" => self.metadata.cell_phone == other,
            "address" => self.metadata.address == other,
            "address2" => self.metadata.address2 == other,
            "city" => self.metadata.city == other,
            "state" => self.metadata.state == other,
            "country" => self.metadata.country == other,
            _ => false,
        }
    }
}

#[derive(Default, Clone, Serialize, Deserialize)]
#[repr(i32)]
pub enum UserRole {
    #[default]
    User = 0,
    Viewer = 10,
    Contributor = 20,
    Editor = 30,
    Manager = 40,
    Admin = 50,
    Custom(String, i32) = -1,
    SuperAdmin = i32::MAX,
}
impl Display for UserRole {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            UserRole::User => f.write_str("User"),
            UserRole::Viewer => f.write_str("Viewer"),
            UserRole::Contributor => f.write_str("Contributor"),
            UserRole::Editor => f.write_str("Editor"),
            UserRole::Manager => f.write_str("Manager"),
            UserRole::Admin => f.write_str("Admin"),
            UserRole::Custom(name, _) => f.write_str(name),
            UserRole::SuperAdmin => f.write_str("SuperAdmin"),
        }
    }
}

#[derive(FieldNamesAsSlice, Default, Clone, Serialize, Deserialize)]
pub struct UserMetaData {
    pub first_name: String,
    pub last_name: String,
    pub home_phone: String,
    pub work_phone: String,
    pub cell_phone: String,
    pub address: String,
    pub address2: String,
    pub city: String,
    pub state: String,
    pub country: String,
    pub notes: String,
}
