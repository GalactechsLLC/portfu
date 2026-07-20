use crate::users::UserRole;
use http::header::{AUTHORIZATION, CONTENT_TYPE};
use http::{HeaderValue, StatusCode};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use log::{debug, info, warn};
use portfu::macros::{get, post};
use portfu::pfcore::router::middleware::{Middleware, MiddlewareImpl, MiddlewareResult};
use portfu::pfcore::services::body::BodyType;
use portfu::pfcore::{Json, Query};
use portfu::prelude::async_trait::async_trait;
use portfu::prelude::http_body_util::Full;
use portfu::prelude::hyper::body::Bytes;
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
#[cfg(feature = "keycloak_auth")]
pub mod keycloak;

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
        session: Arc<RwLock<Session>>,
    ) -> Result<Claims, Error>;
}

#[get("/auth/jwt")]
pub async fn get_jwt(data: &mut ServiceData) -> Result<String, Error> {
    let session = data.request.get::<Arc<RwLock<Session>>>().cloned();
    if let Some(session) = session.as_ref() {
        debug!("Found Session: {}", session.read().await.id);
        if let Some(claims) = session.read().await.data.get::<Claims>() {
            debug!("Found Claims for Session: {}", session.read().await.id);
            return encode_claims(claims);
        }
    }

    *data.response.status_mut() = StatusCode::NOT_FOUND;
    Ok(String::new())
}

#[post("/auth/login")]
pub async fn basic_login<B: BasicAuth + Send + Sync + 'static>(
    login_handle: State<B>,
    session: State<RwLock<Session>>,
    json: Json<Option<BasicLoginRequest>>,
    query: Query<Option<BasicLoginRequest>>,
) -> Result<String, Error> {
    let body: BasicLoginRequest = match json.inner() {
        Some(v) => v,
        None => match query.inner() {
            Some(v) => v,
            None => return Err(Error::other("No Auth Request Found")),
        },
    };
    let claims: Claims = login_handle
        .0
        .as_ref()
        .login(body.username, body.password, session.0.clone())
        .await?;
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

pub static CURRENT_SECRET: Lazy<String> = Lazy::new(|| match env::var("JWT_SECRET") {
    Ok(secret) if !secret.trim().is_empty() => secret,
    _ if require_jwt_secret() => {
        panic!("JWT_SECRET is required when REQUIRE_JWT_SECRET is enabled")
    }
    _ => {
        warn!("JWT_SECRET is unset; falling back to an ephemeral secret");
        Uuid::new_v4().to_string()
    }
});

pub static VALIDATIONS: Lazy<Validation> =
    Lazy::new(|| build_validation(&current_audience(), &current_issuer(), true));

const UNAUTHORIZED_BODY: &[u8] = br#"{"error":"unauthorized"}"#;
const FORBIDDEN_BODY: &[u8] = br#"{"error":"forbidden"}"#;

fn set_auth_error(data: &mut ServiceData, status: StatusCode, body: &'static [u8]) {
    *data.response.status_mut() = status;
    data.response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    data.response
        .set_body(BodyType::Sized(Full::new(Bytes::from_static(body))));
}

fn extract_jwt_from_headers(data: &ServiceData) -> Option<String> {
    if let Some(auth_header) = data.request.headers().get(AUTHORIZATION) {
        if let Ok(value) = auth_header.to_str() {
            let trimmed = value.trim();
            if let Some(token) = trimmed.strip_prefix("Bearer ").map(str::trim) {
                if !token.is_empty() {
                    return Some(token.to_string());
                }
                warn!("Authorization Bearer header was present but token was empty");
            }
        } else {
            warn!("Authorization header was present but not valid UTF-8");
        }
    }

    None
}

fn current_issuer() -> String {
    env::var("JWT_ISSUER")
        .or_else(|_| env::var("KEYCLOAK_ISSUER"))
        .unwrap_or_else(|_| "localhost".to_string())
}

fn require_jwt_secret() -> bool {
    env::var("REQUIRE_JWT_SECRET")
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn current_audience() -> String {
    env::var("JWT_AUDIENCE")
        .or_else(|_| env::var("KEYCLOAK_AUDIENCE"))
        .or_else(|_| env::var("KEYCLOAK_CLIENT_ID"))
        .unwrap_or_else(|_| "localhost".to_string())
}

fn build_validation(audience: &str, issuer: &str, validate_exp: bool) -> Validation {
    let mut val = Validation::default();
    val.set_audience(&[audience]);
    val.set_issuer(&[issuer]);
    val.set_required_spec_claims(&[
        "aud", "exp", "iat", "iat", "iss", "nbf", "sub", "eml", "rol", "org",
    ]);
    val.validate_exp = validate_exp;
    val
}

fn decode_claims(token: &str) -> Result<Claims, jsonwebtoken::errors::Error> {
    decode::<Claims>(
        token,
        &DecodingKey::from_secret(CURRENT_SECRET.as_bytes()),
        &VALIDATIONS,
    )
    .map(|token_data| token_data.claims)
}

fn encode_claims(claims: &Claims) -> Result<String, Error> {
    encode(
        &Header::default(),
        claims,
        &EncodingKey::from_secret(CURRENT_SECRET.as_bytes()),
    )
    .map_err(|e| {
        Error::new(
            ErrorKind::InvalidInput,
            format!("Failed to Encode JWT: {e:?}"),
        )
    })
}

#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct Claims {
    pub aud: String,   // Optional. Audience
    pub exp: usize, // Required (validate_exp defaults to true in validation). Expiration time (as UTC timestamp)
    pub iat: usize, // Optional. Issued at (as UTC timestamp)
    pub iss: String, // Optional. Issuer
    pub nbf: usize, // Optional. Not Before (as UTC timestamp)
    pub sub: String, // Optional. User ID
    pub eml: String, // Optional. User Email
    pub uid: String, // Optional. User Id
    pub rol: UserRole, // Optional. UserRole
    pub org: Vec<u64>, // Optional. UserOrganizations
}

macro_rules! user_role_macro {
    ($variant:ident, $object:ident) => {
        pub struct $object {}
        #[async_trait]
        impl<'a> Middleware for $object {
            fn name(&self) -> &str {
                stringify!($variant)
            }
            async fn before(
                &self,
                data: &mut portfu::pfcore::ServiceData,
            ) -> Result<MiddlewareResult, Error> {
                let path = data.request.uri().path().to_string();
                let has_authorization_header =
                    data.request.headers().get(AUTHORIZATION).is_some();

                let session = data.request.get::<Arc<RwLock<Session>>>().cloned();
                if let Some(session_ref) = session.as_ref() {
                    if let Some(claims) = session_ref.read().await.data.get::<Claims>() {
                        debug!(
                            "Admin auth session claims found path={} role={:?} uid={} sub={}",
                            path, claims.rol, claims.uid, claims.sub
                        );
                        if claims.rol >= UserRole::$object {
                            return Ok(MiddlewareResult::Continue);
                        }
                        warn!(
                            "Admin auth forbidden path={} required_role={:?} actual_role={:?}",
                            path,
                            UserRole::$object,
                            claims.rol
                        );
                        set_auth_error(data, StatusCode::FORBIDDEN, FORBIDDEN_BODY);
                        return Ok(MiddlewareResult::Return);
                    }
                    info!(
                        "Admin auth no session claims path={} authorization_header_present={}",
                        path, has_authorization_header
                    );
                } else {
                    warn!(
                        "Admin auth session state missing path={} authorization_header_present={}",
                        path, has_authorization_header
                    );
                }

                if let Some(jwt_token) = extract_jwt_from_headers(data) {
                    match decode_claims(&jwt_token) {
                        Ok(claims) => {
                            debug!(
                                "Admin auth JWT decoded path={} role={:?} uid={} sub={}",
                                path,
                                claims.rol,
                                claims.uid,
                                claims.sub
                            );

                            if let Some(session_ref) = session.as_ref() {
                                session_ref.write().await.data.insert(claims.clone());
                            }

                            if claims.rol >= UserRole::$object {
                                return Ok(MiddlewareResult::Continue);
                            }

                            warn!(
                                "Admin auth forbidden after JWT decode path={} required_role={:?} actual_role={:?}",
                                path,
                                UserRole::$object,
                                claims.rol
                            );
                            set_auth_error(data, StatusCode::FORBIDDEN, FORBIDDEN_BODY);
                            return Ok(MiddlewareResult::Return);
                        }
                        Err(e) => {
                            warn!("Admin auth JWT decode failed path={} error={e:?}", path);
                        }
                    }
                } else {
                    warn!(
                        "Admin auth missing usable bearer token path={} authorization_header_present={}",
                        path, has_authorization_header
                    );
                }
                warn!(
                    "Admin auth unauthorized path={} authorization_header_present={}",
                    path, has_authorization_header
                );
                set_auth_error(data, StatusCode::UNAUTHORIZED, UNAUTHORIZED_BODY);
                Ok(MiddlewareResult::Return)
            }

            async fn after(
                &self,
                _data: &mut portfu::pfcore::ServiceData,
            ) -> Result<MiddlewareResult, Error> {
                Ok(MiddlewareResult::Continue)
            }
        }
        pub static $variant: Lazy<Arc<MiddlewareImpl>> = Lazy::new(|| {
            Arc::new(MiddlewareImpl {
                name: stringify!($variant).to_string(),
                handlers: vec![Arc::new($object {})],
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_claims() -> Claims {
        Claims {
            aud: "localhost".to_string(),
            exp: 4_102_444_800,
            iat: 1_700_000_000,
            iss: "localhost".to_string(),
            nbf: 1_700_000_000,
            sub: "42".to_string(),
            eml: "user@example.com".to_string(),
            uid: "42".to_string(),
            rol: UserRole::User,
            org: vec![],
        }
    }

    #[test]
    fn build_validation_uses_supplied_audience_and_issuer() {
        let validation = build_validation("nebula-dev", "https://issuer.example", false);
        assert_eq!(
            validation.aud,
            Some(std::collections::HashSet::from(["nebula-dev".to_string()]))
        );
        assert_eq!(
            validation.iss,
            Some(std::collections::HashSet::from([
                "https://issuer.example".to_string()
            ]))
        );
        assert!(!validation.validate_exp);
    }

    #[test]
    fn decode_claims_accepts_locally_issued_token() {
        let claims = sample_claims();
        let token = encode_claims(&claims).expect("token should encode");
        let decoded = decode_claims(&token).expect("token should decode");
        assert_eq!(decoded.sub, "42");
        assert_eq!(decoded.uid, "42");
        assert_eq!(decoded.eml, "user@example.com");
    }
}
