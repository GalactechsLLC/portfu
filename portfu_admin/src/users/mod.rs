use portfu::prelude::uuid::Uuid;
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
#[cfg(feature = "postgres")]
use sqlx::{Error, FromRow, Postgres, Row};
#[cfg(feature = "postgres")]
use sqlx::database::{HasArguments};
#[cfg(feature = "postgres")]
use sqlx::postgres::{PgRow};
#[cfg(feature = "postgres")]
use sqlx::query::Query;
use struct_field_names_as_array::FieldNamesAsSlice;
use time::OffsetDateTime;
#[cfg(feature = "postgres")]
use crate::stores::{DatabaseEntry};
use crate::stores::{DataStoreEntry};

#[derive(FieldNamesAsSlice, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct User {
    pub id: i64,
    pub uuid: Uuid,
    pub username: String,
    pub email: String,
    pub role: UserRole,
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
            first_name: Default::default(),
            last_name: Default::default(),
            home_phone: Default::default(),
            work_phone: Default::default(),
            cell_phone: Default::default(),
            address: Default::default(),
            address2: Default::default(),
            city: Default::default(),
            state: Default::default(),
            country: Default::default(),
            notes: Default::default(),
            created: OffsetDateTime::now_utc(),
            updated: OffsetDateTime::now_utc(),
        }
    }
}
impl DataStoreEntry<i64> for User {
    fn key_name() -> &'static str {
        "id"
    }
    fn key_value(&self) -> i64 {
        self.id
    }

    fn parameters() -> &'static [&'static str] {
        User::FIELD_NAMES_AS_SLICE
    }

    fn matches(&self, name: &str, other: &str) -> bool {
        if !Self::FIELD_NAMES_AS_SLICE.contains(&name) {
            return false;
        }
        match name {
            "id" => self.id.to_string() == other,
            "uuid" => self.uuid.to_string() == other,
            "username" => self.username == other,
            "email" => self.email == other,
            "role" => self.role.to_string().to_ascii_lowercase() == other.to_ascii_lowercase(),
            "first_name" => self.first_name == other,
            "last_name" => self.last_name == other,
            "home_phone" => self.home_phone == other,
            "work_phone" => self.work_phone == other,
            "cell_phone" => self.cell_phone == other,
            "address" => self.address == other,
            "address2" => self.address2 == other,
            "city" => self.city == other,
            "state" => self.state == other,
            "country" => self.country == other,
            _ => false,
        }
    }
}

#[cfg(feature = "postgres")]
impl<'r> FromRow<'r, PgRow> for User {
    fn from_row(row: &'r PgRow) -> Result<Self, Error> {
        Ok(Self {
            id: row.try_get("id")?,
            uuid: Uuid::parse_str(row.try_get("uuid")?).map_err(|e| {
                Error::Decode(e.into())
            })?,
            username: row.try_get("username")?,
            email: row.try_get("email")?,
            role: row.try_get("role")?,
            first_name: row.try_get("first_name")?,
            last_name: row.try_get("last_name:")?,
            home_phone: row.try_get("home_phone")?,
            work_phone: row.try_get("work_phone")?,
            cell_phone: row.try_get("cell_phone")?,
            address:  row.try_get("address")?,
            address2: row.try_get("address2")?,
            city: row.try_get("city")?,
            state: row.try_get("state")?,
            country: row.try_get("country")?,
            notes: row.try_get("notes")?,
            created: row.try_get("created")?,
            updated: row.try_get("updated")?,
        })
    }
}

#[cfg(feature = "postgres")]
impl DatabaseEntry<PgRow, i64> for User {
    fn bind<'q>(&'q self, mut query: Query<'q, Postgres, <Postgres as HasArguments>::Arguments>, field: &str) -> Query<'q, Postgres, <Postgres as HasArguments>::Arguments> {
        if !Self::FIELD_NAMES_AS_SLICE.contains(&field)
        {
            return query;
        }
        query = match field {
            "id" => query.bind(self.id),
            "uuid" =>query.bind(self.uuid.to_string()),
            "username" => query.bind(&self.username),
            "email" => query.bind(&self.email),
            "role" => query.bind(self.role.to_string()),
            "first_name" => query.bind(&self.first_name),
            "last_name" => query.bind(&self.last_name),
            "home_phone" => query.bind(&self.home_phone),
            "work_phone" => query.bind(&self.work_phone),
            "cell_phone" => query.bind(&self.cell_phone),
            "address" => query.bind(&self.address),
            "address2" => query.bind(&self.address2),
            "city" => query.bind(&self.city),
            "state" => query.bind(&self.state),
            "country" => query.bind(&self.country),
            "notes" => query.bind(&self.notes),
            "created" => query.bind(self.created),
            "updated" => query.bind(self.updated),
            _ => query,
        };
        query
    }

    fn database() -> String {
        "PfAdmin".to_string()
    }

    fn table() -> String {
        "Users".to_string()
    }
}

#[derive(Default, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "postgres", derive(sqlx::Type))]
#[repr(i64)]
pub enum UserRole {
    #[default]
    User = 0,
    Viewer = 10,
    Contributor = 20,
    Editor = 30,
    Manager = 40,
    Admin = 50,
    SuperAdmin = i64::MAX,
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
            UserRole::SuperAdmin => f.write_str("SuperAdmin"),
        }
    }
}
