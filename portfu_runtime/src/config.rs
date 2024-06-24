use portfu_admin::users::UserRole;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use std::io::{Error, ErrorKind};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::{env, fs};

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum DatabaseType {
    Mysql,
    Postgres,
}

impl FromStr for DatabaseType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "mysql" => Ok(DatabaseType::Mysql),
            "postgres" => Ok(DatabaseType::Postgres),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Default, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct OAuthSettings {
    pub client_id: String,
    pub client_secret: String,
    pub auth_url: String,
    pub token_url: String,
    pub redirect_url: String,
    pub user_roles: Vec<(String, Option<UserRole>)>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct Config {
    pub hostname: String,
    pub port: u16,
    pub database_url: Option<String>,
    pub database_type: Option<DatabaseType>,
    pub directories: Vec<String>,
    pub oauth_settings: Option<OAuthSettings>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            hostname: "localhost".to_string(),
            port: 8080,
            database_url: None,
            database_type: None,
            directories: vec![],
            oauth_settings: None,
        }
    }
}

impl Config {
    pub fn from_env() -> Self {
        let hostname = env::var("HOSTNAME").unwrap_or_else(|_| "localhost".to_string());
        let port = env::var("PORT")
            .ok()
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(8080);
        let database_url = env::var("DATABASE_URL").ok();
        let database_type = match &database_url {
            Some(url) => {
                if url.starts_with("mysql://") {
                    Some(DatabaseType::Mysql)
                } else if url.starts_with("postgres://") {
                    Some(DatabaseType::Postgres)
                } else {
                    None
                }
            }
            None => None,
        };
        let directories = env::var("DIRECTORIES")
            .ok()
            .map_or(vec![], |s| s.split(',').map(String::from).collect());
        let oauth_settings = if let (
            Some(client_id),
            Some(client_secret),
            Some(auth_url),
            Some(token_url),
            Some(redirect_url),
        ) = (
            env::var("OAUTH_CLIENT_ID").ok(),
            env::var("OAUTH_CLIENT_SECRET").ok(),
            env::var("OAUTH_AUTH_URL").ok(),
            env::var("OAUTH_TOKEN_URL").ok(),
            env::var("OAUTH_REDIRECT_URL").ok(),
        ) {
            Some(OAuthSettings {
                client_id,
                client_secret,
                auth_url,
                token_url,
                redirect_url,
                user_roles: env::var("USER_ROLES").ok().map_or(vec![], |s| {
                    s.split(',')
                        .map(|s| {
                            let split: Vec<&str> = s.split(':').collect();
                            if split.len() == 2 {
                                (split[0].to_string(), UserRole::from_str(split[1]).ok())
                            } else {
                                (s.to_string(), None)
                            }
                        })
                        .collect()
                }),
            })
        } else {
            None
        };
        Config {
            hostname,
            port,
            database_url,
            database_type,
            directories,
            oauth_settings,
        }
    }
}
impl TryFrom<&Path> for Config {
    type Error = Error;
    fn try_from(value: &Path) -> Result<Self, Self::Error> {
        serde_yaml::from_str::<Config>(&fs::read_to_string(value)?)
            .map_err(|e| Error::new(ErrorKind::Other, format!("{:?}", e)))
    }
}
impl TryFrom<&PathBuf> for Config {
    type Error = Error;
    fn try_from(value: &PathBuf) -> Result<Self, Self::Error> {
        Self::try_from(value.as_path())
    }
}
