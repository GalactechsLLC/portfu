use crate::users::UserRole;
use http::StatusCode;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use portfu::macros::{get, post};
use portfu::pfcore::wrappers::{Wrapper, WrapperFn, WrapperResult};
use portfu::pfcore::{FromRequest, Json, Query};
use portfu::prelude::async_trait::async_trait;
use portfu::prelude::log::error;
use portfu::prelude::once_cell::sync::Lazy;
use portfu::prelude::uuid::Uuid;
use portfu::prelude::{ServiceData, State};
use portfu::wrappers::sessions::Session;
use serde::{Deserialize, Serialize};
use std::env;
use std::io::{Error, ErrorKind};
use std::sync::Arc;
use tokio::sync::RwLock;

#[cfg(feature = "github_auth")]
pub mod github;

#[derive(Default, Clone, Deserialize)]
pub struct BasicLoginRequest {
    username: String,
    password: String,
}

#[async_trait]
pub trait BasicAuth {
    async fn login<U: AsRef<str> + Send + Sync, P: AsRef<str> + Send + Sync>(
        &self,
        username: U,
        password: P,
    ) -> Result<Claims, Error>;
}

#[get("/auth/jwt")]
pub async fn get_jwt(data: &mut ServiceData) -> Result<String, Error> {
    if let Some(session) = data.request.get::<Arc<RwLock<Session>>>() {
        if let Some(claims) = session.read().await.data.get::<Claims>() {
            return encode(
                &Header::default(),
                claims,
                &EncodingKey::from_secret(CURRENT_SECRET.as_bytes()),
            )
            .map_err(|e| {
                Error::new(
                    ErrorKind::InvalidInput,
                    format!("Failed to Encode JWT: {e:?}"),
                )
            });
        }
    }
    *data.response.status_mut() = StatusCode::NOT_FOUND;
    Ok(String::new())
}

#[post("/auth/login")]
pub async fn basic_login<B: BasicAuth + Send + Sync + 'static>(
    data: &mut ServiceData,
) -> Result<String, Error> {
    let basic_login: Arc<B> = match data.request.get::<Arc<B>>() {
        Some(data) => data.clone(),
        None => {
            let msg = "No Auth Backend for to handle Request";
            error!("{}", msg);
            *data.response.status_mut() = StatusCode::NOT_FOUND;
            return Ok(msg.to_string());
        }
    };
    let body: Option<BasicLoginRequest> = match Json::from_request(&mut data.request, "").await {
        Ok(json) => json.inner(),
        Err(_) => None,
    };
    let body: BasicLoginRequest = match body {
        None => Query::<BasicLoginRequest>::from_request(&mut data.request, "")
            .await
            .map(|v| v.inner())?,
        Some(v) => v,
    };
    let claims: Claims = match basic_login
        .as_ref()
        .login(body.username, body.password)
        .await
    {
        Ok(claims) => claims,
        Err(_) => {
            *data.response.status_mut() = StatusCode::BAD_REQUEST;
            return Err(Error::new(ErrorKind::InvalidInput, "Invalid Login"));
        }
    };
    if let Ok(session) = State::<Arc<RwLock<Session>>>::from_request(&mut data.request, "").await {
        session.0.write().await.data.insert(claims.clone());
    }
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(CURRENT_SECRET.as_bytes()),
    )
    .map_err(|e| {
        Error::new(
            ErrorKind::InvalidInput,
            format!("Failed to Encode JWT: {e:?}"),
        )
    })
}

pub static CURRENT_SECRET: Lazy<String> =
    Lazy::new(|| env::var("JWT_SECRET").unwrap_or_else(|_| Uuid::new_v4().to_string()));

pub static VALIDATIONS: Lazy<Validation> = Lazy::new(|| {
    let mut val = Validation::default();
    val.set_audience(&["localhost"]);
    val.set_issuer(&["localhost"]);
    val.set_required_spec_claims(&[
        "aud", "exp", "iat", "iat", "iss", "nbf", "sub", "eml", "rol", "org",
    ]);
    val.validate_exp = false;
    val
});

#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct Claims {
    pub aud: String,   // Optional. Audience
    pub exp: usize, // Required (validate_exp defaults to true in validation). Expiration time (as UTC timestamp)
    pub iat: usize, // Optional. Issued at (as UTC timestamp)
    pub iss: String, // Optional. Issuer
    pub nbf: usize, // Optional. Not Before (as UTC timestamp)
    pub sub: String, // Optional. User ID
    pub eml: String, // Optional. User Email
    pub rol: UserRole, // Optional. UserRole
    pub org: Vec<u64>, // Optional. UserOrganizations
}

macro_rules! user_role_macro {
    ($variant:ident, $object:ident) => {
        pub struct $object {}
        #[async_trait]
        impl<'a> WrapperFn for $object {
            fn name(&self) -> &str {
                stringify!($variant)
            }
            async fn before(&self, data: &mut portfu::pfcore::ServiceData) -> WrapperResult {
                if let Some(session) = data.request.get::<Arc<RwLock<Session>>>() {
                    if let Some(claims) = session.read().await.data.get::<Claims>() {
                        if claims.rol >= UserRole::$object {
                            return WrapperResult::Continue;
                        }
                    } else {
                        if let Some(headers) = data.request.request.headers() {
                            if let Some(jwt_header) = headers.get("USER_JWT") {
                                if let Ok(str_val) = jwt_header.to_str() {
                                    match decode::<Claims>(
                                        str_val,
                                        &DecodingKey::from_secret(CURRENT_SECRET.as_bytes()),
                                        &*VALIDATIONS,
                                    ) {
                                        Ok(token_data) => {
                                            let res =
                                                (token_data.claims.rol >= UserRole::$object).into();
                                            session.write().await.data.insert(token_data.claims);
                                            return res;
                                        }
                                        Err(e) => {
                                            error!("Error Parsing JWT Token: {e:?}");
                                        }
                                    };
                                }
                            }
                        }
                    }
                }
                WrapperResult::Return
            }

            async fn after(&self, _data: &mut portfu::pfcore::ServiceData) -> WrapperResult {
                WrapperResult::Continue
            }
        }
        pub static $variant: Lazy<Arc<Wrapper>> = Lazy::new(|| {
            Arc::new(Wrapper {
                name: stringify!($variant).to_string(),
                wrapper_functions: vec![Arc::new($object {})],
            })
        });
    };
}

user_role_macro!(USER, User);
user_role_macro!(VIEWER, Viewer);
user_role_macro!(CONTRIBUTOR, Contributor);
user_role_macro!(EDITOR, Editor);
user_role_macro!(MANAGER, Manager);
user_role_macro!(ADMIN, Admin);
user_role_macro!(SUPERADMIN, SuperAdmin);
