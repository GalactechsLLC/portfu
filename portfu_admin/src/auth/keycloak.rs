use crate::auth::Claims;
use crate::services::{redirect_to_url, sanitize_relative_redirect_target, send_internal_error};
use crate::users::UserRole;
use base64::Engine;
use http::HeaderValue;
use hyper::{header, StatusCode};
use log::{debug, info, warn};
use oauth2::basic::BasicClient;
use oauth2::reqwest::async_http_client;
use oauth2::{
    AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, Scope, TokenResponse, TokenUrl,
};
use portfu::pfcore::services::builder::ServiceBuilder;
use portfu::pfcore::services::group::ServiceGroup;
use portfu::pfcore::{FromRequest, Json, Query, ServiceData, ServiceHandler, ServiceType};
use portfu::prelude::async_trait;
use portfu::wrappers::sessions::Session;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::env;
use std::future::Future;
use std::io::{Error, ErrorKind};
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use time::OffsetDateTime;
use tokio::sync::RwLock;

pub struct OAuthConfig {
    pub client: BasicClient,
    pub client_id: ClientId,
    pub client_secret: ClientSecret,
    pub issuer_url: String,
    pub auth_url: AuthUrl,
    pub token_url: TokenUrl,
    pub userinfo_url: String,
    pub on_success_redirect: String,
    pub on_failure_redirect: String,
    pub claims_audience: String,
    pub claims_issuer: String,
    pub claims_expire_time: usize,
    pub require_verified_email: bool,
    pub default_role: UserRole,
    pub allowed_roles: Vec<String>,
    pub admin_roles: Vec<String>,
    pub callbacks: Vec<OAuthCallbackFn>,
}

#[derive(Default, Clone, Deserialize)]
pub struct AuthRequest {
    #[serde(default)]
    code: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    error: String,
    #[serde(default)]
    error_description: String,
}

#[derive(Default, Clone, Deserialize)]
struct RoleCollection {
    #[serde(default)]
    roles: Vec<String>,
}

#[derive(Default, Clone, Deserialize)]
struct UserInfo {
    #[serde(default)]
    sub: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    email_verified: bool,
    #[serde(default)]
    preferred_username: String,
    #[serde(default)]
    realm_access: Option<RoleCollection>,
    #[serde(default)]
    resource_access: HashMap<String, RoleCollection>,
}

#[derive(Default, Clone, Deserialize)]
struct AccessTokenClaims {
    #[serde(default)]
    realm_access: Option<RoleCollection>,
    #[serde(default)]
    resource_access: HashMap<String, RoleCollection>,
}

fn resolve_authorized_role(
    default_role: UserRole,
    extracted_roles: &HashSet<String>,
    admin_roles: &HashSet<String>,
    allowed_roles: &HashSet<String>,
) -> UserRole {
    if !admin_roles.is_empty()
        && extracted_roles
            .iter()
            .any(|role| admin_roles.contains(role))
    {
        return UserRole::Admin;
    }

    if !allowed_roles.is_empty()
        && extracted_roles
            .iter()
            .any(|role| allowed_roles.contains(role))
    {
        return if default_role == UserRole::None {
            UserRole::User
        } else {
            default_role
        };
    }

    UserRole::None
}

fn sorted_strings(values: &HashSet<String>) -> Vec<String> {
    let mut values: Vec<String> = values.iter().cloned().collect();
    values.sort();
    values
}

fn extract_roles_from_collections(
    realm_access: &Option<RoleCollection>,
    resource_access: &HashMap<String, RoleCollection>,
) -> HashSet<String> {
    let mut extracted_roles: HashSet<String> = HashSet::new();
    if let Some(realm_access) = realm_access {
        for role in &realm_access.roles {
            if !role.is_empty() {
                extracted_roles.insert(role.to_ascii_lowercase());
            }
        }
    }
    for resource in resource_access.values() {
        for role in &resource.roles {
            if !role.is_empty() {
                extracted_roles.insert(role.to_ascii_lowercase());
            }
        }
    }
    extracted_roles
}

fn decode_jwt_payload(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    let normalized = payload.replace('-', "+").replace('_', "/");
    let padding = (4 - normalized.len() % 4) % 4;
    let padded = format!("{normalized}{}", "=".repeat(padding));
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(padded)
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn extract_roles_from_access_token(token: &str) -> Option<HashSet<String>> {
    let claims: AccessTokenClaims = serde_json::from_value(decode_jwt_payload(token)?).ok()?;
    Some(extract_roles_from_collections(
        &claims.realm_access,
        &claims.resource_access,
    ))
}

fn trusted_userinfo_email(user_info: &UserInfo, require_verified_email: bool) -> Option<String> {
    let email = user_info.email.trim();
    if !email.is_empty() && (!require_verified_email || user_info.email_verified) {
        Some(email.to_string())
    } else {
        None
    }
}

fn resolved_subject(user_info: &UserInfo, trusted_email: Option<&str>) -> String {
    let preferred_username = user_info.preferred_username.trim();
    if !preferred_username.is_empty() {
        preferred_username.to_string()
    } else if let Some(email) = trusted_email {
        email.to_string()
    } else {
        user_info.sub.trim().to_string()
    }
}

pub enum CallbackResult {
    Continue(ServiceData),
    Return(ServiceData),
}

pub type ErrCallbackFn = Pin<
    Box<
        dyn Fn(
                ServiceData,
            )
                -> Pin<Box<dyn Future<Output = Result<CallbackResult, (ServiceData, Error)>> + Send>>
            + Send
            + Sync
            + 'static,
    >,
>;
pub type CallbackFn = Pin<
    Box<
        dyn Fn(
                Claims,
                ServiceData,
            )
                -> Pin<Box<dyn Future<Output = Result<CallbackResult, (ServiceData, Error)>> + Send>>
            + Send
            + Sync
            + 'static,
    >,
>;

pub enum OAuthCallbackFn {
    OnSuccess(CallbackFn),
    OnFailure(ErrCallbackFn),
}

pub struct OAuthLoginHandler {
    config: Arc<OAuthConfig>,
}

#[derive(Clone)]
struct Verifier(String);

#[derive(Deserialize, Serialize, Clone)]
pub struct OAuthLoginRedirectParams {
    redirect_url: String,
}

#[async_trait::async_trait]
impl ServiceHandler for OAuthLoginHandler {
    fn name(&self) -> &str {
        "login"
    }
    async fn handle(
        &self,
        mut data: portfu::prelude::ServiceData,
    ) -> Result<ServiceData, (ServiceData, Error)> {
        let redirect_params = if let Ok(Some(q)) =
            Query::<Option<OAuthLoginRedirectParams>>::from_request(&mut data.request, "")
                .await
                .map(|q| q.inner())
        {
            sanitize_relative_redirect_target(&q.redirect_url)
                .map(|redirect_url| OAuthLoginRedirectParams { redirect_url })
        } else {
            None
        };
        let session = if let Some(session) = data.request.get::<Arc<RwLock<Session>>>().cloned() {
            session
        } else {
            warn!("Failed to find session for OAuth login");
            return Ok(send_internal_error(data, "Failed to find session to auth"));
        };
        let (pkce_code_challenge, pkce_code_verifier) = PkceCodeChallenge::new_random_sha256();
        session
            .write()
            .await
            .data
            .insert(Verifier(pkce_code_verifier.secret().to_string()));
        let auth_request = self
            .config
            .client
            .authorize_url(CsrfToken::new_random)
            .add_scope(Scope::new("openid".into()))
            .add_scope(Scope::new("profile".into()))
            .add_scope(Scope::new("email".into()))
            .set_pkce_challenge(pkce_code_challenge);
        if let Some(redirect_params) = redirect_params {
            session.write().await.data.insert(redirect_params);
        }
        let (auth_url, csrf_token) = auth_request.url();
        session.write().await.data.insert(csrf_token);
        *data.response.status_mut() = StatusCode::FOUND;
        data.response.headers_mut().insert(
            header::LOCATION,
            HeaderValue::from_str(auth_url.as_str()).unwrap_or(HeaderValue::from_static("/")),
        );
        Ok(data)
    }

    fn service_type(&self) -> ServiceType {
        ServiceType::API
    }
}

pub struct OAuthAuthHandler {
    config: Arc<OAuthConfig>,
}

impl OAuthAuthHandler {
    async fn handle_error(
        &self,
        mut data: ServiceData,
        error: &str,
    ) -> Result<ServiceData, (ServiceData, Error)> {
        for callback in &self.config.callbacks {
            if let OAuthCallbackFn::OnFailure(err_fn) = callback {
                match (*err_fn)(data).await? {
                    CallbackResult::Continue(d) => data = d,
                    CallbackResult::Return(d) => return Ok(d),
                }
            }
        }
        Ok(send_internal_error(data, error))
    }

    async fn handle_failure(
        &self,
        mut data: ServiceData,
        url: &str,
    ) -> Result<ServiceData, (ServiceData, Error)> {
        for callback in &self.config.callbacks {
            if let OAuthCallbackFn::OnFailure(err_fn) = callback {
                match (*err_fn)(data).await? {
                    CallbackResult::Continue(d) => data = d,
                    CallbackResult::Return(d) => return Ok(d),
                }
            }
        }
        Ok(redirect_to_url(data, url))
    }

    async fn handle_success(
        &self,
        mut data: ServiceData,
        claims: Claims,
        url: &str,
    ) -> Result<ServiceData, (ServiceData, Error)> {
        for callback in &self.config.callbacks {
            if let OAuthCallbackFn::OnSuccess(callback) = callback {
                match callback(claims.clone(), data).await? {
                    CallbackResult::Continue(d) => data = d,
                    CallbackResult::Return(d) => return Ok(d),
                }
            }
        }
        Ok(redirect_to_url(data, url))
    }
}

#[async_trait::async_trait]
impl ServiceHandler for OAuthAuthHandler {
    fn name(&self) -> &str {
        "auth"
    }
    async fn handle(
        &self,
        mut data: portfu::prelude::ServiceData,
    ) -> Result<ServiceData, (ServiceData, Error)> {
        let session = if let Some(session) = data.request.get::<Arc<RwLock<Session>>>() {
            session.clone()
        } else {
            warn!("Failed to find session for OAuth callback");
            return self
                .handle_error(data, "Failed to find session to auth")
                .await;
        };
        let body: Option<AuthRequest> = match Json::from_request(&mut data.request, "").await {
            Ok(json) => json.inner(),
            Err(_) => None,
        };
        let body: AuthRequest = match body {
            None => match Query::<Option<AuthRequest>>::from_request(&mut data.request, "").await {
                Ok(v) => match v.inner() {
                    Some(v) => v,
                    None => {
                        warn!("Failed to extract OAuth callback request");
                        return self
                            .handle_error(data, "Failed to extract OAuth callback request")
                            .await;
                    }
                },
                Err(e) => {
                    warn!("Failed to extract OAuth callback query");
                    return self
                        .handle_error(
                            data,
                            &format!("Failed to extract query as OAuth callback request, {e:?}"),
                        )
                        .await;
                }
            },
            Some(v) => v,
        };
        if !body.error.trim().is_empty() {
            warn!(
                "OAuth callback returned provider error={} description={}",
                body.error, body.error_description
            );
            return self
                .handle_failure(data, self.config.on_failure_redirect.as_str())
                .await;
        }
        if body.code.trim().is_empty() || body.state.trim().is_empty() {
            warn!(
                "OAuth callback missing code or state query={:?}",
                data.request.uri().query()
            );
            return self
                .handle_failure(data, self.config.on_failure_redirect.as_str())
                .await;
        }
        let existing_token = if let Some(token) = session.read().await.data.get::<CsrfToken>() {
            token.clone()
        } else {
            warn!("Failed to find csrf token");
            return self.handle_error(data, "Failed to find csrf token").await;
        };
        let token_state = CsrfToken::new(body.state.clone());
        if existing_token.secret() != token_state.secret() {
            warn!("Invalid csrf token");
            return self.handle_error(data, "Invalid csrf token").await;
        }
        let verifier = if let Some(verifier) = session.read().await.data.get::<Verifier>() {
            PkceCodeVerifier::new(verifier.0.clone())
        } else {
            warn!("Failed to find PKCE verifier");
            return self
                .handle_error(data, "Failed to find PKCE verifier")
                .await;
        };
        let code = AuthorizationCode::new(body.code.clone());
        let token = match self
            .config
            .client
            .exchange_code(code)
            .set_pkce_verifier(verifier)
            .request_async(async_http_client)
            .await
        {
            Ok(token) => token,
            Err(e) => {
                warn!("Failed to exchange OAuth code for token: {e:?}");
                return self
                    .handle_failure(data, self.config.on_failure_redirect.as_str())
                    .await;
            }
        };
        let access_token = token.access_token().secret().to_string();
        let token_val = format!("Bearer {}", access_token);
        let client = match reqwest::Client::builder().build() {
            Ok(client) => client,
            Err(e) => {
                return Err((
                    data,
                    Error::other(format!("Failed to build HTTP client for userinfo: {e:?}")),
                ));
            }
        };
        let user_info: UserInfo = match client
            .get(&self.config.userinfo_url)
            .header("Authorization", &token_val)
            .header("Accept", "application/json")
            .send()
            .await
        {
            Ok(resp) => {
                if !resp.status().is_success() {
                    warn!("Failed to load user info: HTTP {}", resp.status());
                    return self
                        .handle_failure(data, self.config.on_failure_redirect.as_str())
                        .await;
                }
                match resp.json().await {
                    Ok(info) => info,
                    Err(e) => {
                        warn!("Failed to parse user info response: {e:?}");
                        return self
                            .handle_failure(data, self.config.on_failure_redirect.as_str())
                            .await;
                    }
                }
            }
            Err(e) => {
                warn!("Failed to call user info endpoint: {e:?}");
                return self
                    .handle_failure(data, self.config.on_failure_redirect.as_str())
                    .await;
            }
        };
        let mut claims: Claims = session.read().await.data.get().cloned().unwrap_or(Claims {
            aud: self.config.claims_audience.clone(),
            exp: OffsetDateTime::now_utc().unix_timestamp() as usize
                + self.config.claims_expire_time,
            iat: OffsetDateTime::now_utc().unix_timestamp() as usize,
            iss: self.config.claims_issuer.clone(),
            nbf: OffsetDateTime::now_utc().unix_timestamp() as usize,
            sub: "".to_string(),
            eml: "".to_string(),
            uid: "".to_string(),
            rol: self.config.default_role,
            org: vec![],
        });
        let trusted_email = trusted_userinfo_email(&user_info, self.config.require_verified_email);
        claims.uid = user_info.sub.clone();
        claims.eml = trusted_email.clone().unwrap_or_default();
        claims.sub = resolved_subject(&user_info, trusted_email.as_deref());
        let mut extracted_roles =
            extract_roles_from_collections(&user_info.realm_access, &user_info.resource_access);
        let mut used_access_token_role_fallback = false;
        if extracted_roles.is_empty() {
            if let Some(token_roles) = extract_roles_from_access_token(&access_token) {
                if !token_roles.is_empty() {
                    extracted_roles = token_roles;
                    used_access_token_role_fallback = true;
                }
            }
        }
        let admin_roles: HashSet<String> = self
            .config
            .admin_roles
            .iter()
            .map(|v| v.to_ascii_lowercase())
            .collect();
        let allowed_roles: HashSet<String> = self
            .config
            .allowed_roles
            .iter()
            .map(|v| v.to_ascii_lowercase())
            .collect();
        info!(
            "Keycloak userinfo resolved sub={} eml={} email_verified={} require_verified_email={} preferred_username={} extracted_roles={:?} used_access_token_role_fallback={} admin_roles={:?} allowed_roles={:?} default_role={:?}",
            claims.uid,
            claims.eml,
            user_info.email_verified,
            self.config.require_verified_email,
            claims.sub,
            sorted_strings(&extracted_roles),
            used_access_token_role_fallback,
            sorted_strings(&admin_roles),
            sorted_strings(&allowed_roles),
            self.config.default_role,
        );
        claims.rol = resolve_authorized_role(
            self.config.default_role,
            &extracted_roles,
            &admin_roles,
            &allowed_roles,
        );
        if claims.rol == UserRole::None {
            warn!(
                "Keycloak authorization denied sub={} eml={} email_verified={} require_verified_email={} preferred_username={} extracted_roles={:?} used_access_token_role_fallback={} admin_roles={:?} allowed_roles={:?} default_role={:?}",
                claims.uid,
                claims.eml,
                user_info.email_verified,
                self.config.require_verified_email,
                claims.sub,
                sorted_strings(&extracted_roles),
                used_access_token_role_fallback,
                sorted_strings(&admin_roles),
                sorted_strings(&allowed_roles),
                self.config.default_role,
            );
        } else {
            info!(
                "Keycloak authorization granted sub={} eml={} preferred_username={} resolved_role={:?}",
                claims.uid,
                claims.eml,
                claims.sub,
                claims.rol,
            );
        }
        session.write().await.data.insert(claims.clone());
        info!("Running OAuth success handlers");
        let maybe_redirect = session
            .write()
            .await
            .data
            .remove::<OAuthLoginRedirectParams>();
        let url = if let Some(redirect) = maybe_redirect {
            sanitize_relative_redirect_target(&redirect.redirect_url)
                .unwrap_or_else(|| self.config.on_success_redirect.clone())
        } else {
            self.config.on_success_redirect.clone()
        };
        self.handle_success(data, claims, &url).await
    }

    fn service_type(&self) -> ServiceType {
        ServiceType::API
    }
}

#[derive(Default)]
pub struct OAuthLoginBuilder {
    pub client_id: Option<ClientId>,
    pub client_secret: Option<ClientSecret>,
    pub issuer_url: Option<String>,
    pub auth_url: Option<AuthUrl>,
    pub token_url: Option<TokenUrl>,
    pub userinfo_url: Option<String>,
    pub redirect_url: Option<RedirectUrl>,
    pub on_success_redirect: Option<String>,
    pub on_failure_redirect: Option<String>,
    pub claims_audience: Option<String>,
    pub claims_issuer: Option<String>,
    pub claims_expire_time: Option<usize>,
    pub require_verified_email: Option<bool>,
    pub default_role: Option<UserRole>,
    pub callbacks: Vec<OAuthCallbackFn>,
    pub allowed_roles: Vec<String>,
    pub admin_roles: Vec<String>,
}

impl OAuthLoginBuilder {
    pub fn from_env() -> Option<Self> {
        let issuer_url = match env::var("KEYCLOAK_ISSUER_URL") {
            Ok(server) => server.trim_end_matches('/').to_string(),
            Err(e) => {
                warn!("Failed to load KEYCLOAK_ISSUER_URL: {}", e);
                return None;
            }
        };
        let client_id = match env::var("KEYCLOAK_CLIENT_ID") {
            Ok(s) => ClientId::new(s),
            Err(e) => {
                warn!("Failed to load KEYCLOAK_CLIENT_ID: {}", e);
                return None;
            }
        };
        let client_secret = match env::var("KEYCLOAK_CLIENT_SECRET") {
            Ok(s) => ClientSecret::new(s),
            Err(e) => {
                warn!("Failed to load KEYCLOAK_CLIENT_SECRET: {}", e);
                return None;
            }
        };
        let auth_url = match AuthUrl::new(format!("{issuer_url}/protocol/openid-connect/auth")) {
            Ok(u) => u,
            Err(e) => {
                warn!("Failed to parse auth URL: {}", e);
                return None;
            }
        };
        let token_url = match TokenUrl::new(format!("{issuer_url}/protocol/openid-connect/token")) {
            Ok(u) => u,
            Err(e) => {
                warn!("Failed to parse token URL: {}", e);
                return None;
            }
        };
        let userinfo_url = format!("{issuer_url}/protocol/openid-connect/userinfo");
        let redirect_str = match env::var("KEYCLOAK_REDIRECT_URL") {
            Ok(server) => server,
            Err(e) => {
                warn!("Failed to load KEYCLOAK_REDIRECT_URL: {}", e);
                return None;
            }
        };
        let redirect_url = match RedirectUrl::new(redirect_str) {
            Ok(u) => u,
            Err(e) => {
                warn!("Failed to parse redirect URL: {}", e);
                return None;
            }
        };
        let default_role = env::var("KEYCLOAK_DEFAULT_ROLE")
            .ok()
            .and_then(|v| UserRole::from_str(v.trim()).ok())
            .unwrap_or(UserRole::None);
        let require_verified_email = env::var("KEYCLOAK_REQUIRE_VERIFIED_EMAIL")
            .ok()
            .and_then(|v| match v.trim().to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" | "on" => Some(true),
                "false" | "0" | "no" | "off" => Some(false),
                _ => None,
            })
            .unwrap_or(true);
        Some(
            OAuthLoginBuilder::new()
                .client_id(client_id.clone())
                .client_secret(client_secret)
                .issuer_url(issuer_url.clone())
                .auth_url(auth_url)
                .token_url(token_url)
                .userinfo_url(userinfo_url)
                .redirect_url(redirect_url)
                .on_success_redirect(
                    env::var("KEYCLOAK_SUCCESS_URL").unwrap_or_else(|_| String::from("/")),
                )
                .on_failure_redirect(
                    env::var("KEYCLOAK_FAILURE_URL").unwrap_or_else(|_| String::from("/")),
                )
                .claims_issuer(env::var("KEYCLOAK_ISSUER").unwrap_or_else(|_| issuer_url.clone()))
                .claims_audience(
                    env::var("KEYCLOAK_AUDIENCE")
                        .unwrap_or_else(|_| client_id.as_str().to_string()),
                )
                .claims_expire_time(
                    env::var("KEYCLOAK_EXPIRE_TIME")
                        .map(|s| s.parse().unwrap_or(30usize * 60usize))
                        .unwrap_or(30usize * 60usize),
                )
                .require_verified_email(require_verified_email)
                .default_role(default_role)
                .allowed_roles(&csv_env("KEYCLOAK_ALLOWED_ROLES"))
                .admin_roles(&csv_env("KEYCLOAK_ADMIN_ROLES")),
        )
    }

    pub fn new() -> Self {
        Default::default()
    }

    pub fn client_id(self, client_id: ClientId) -> Self {
        let mut s = self;
        s.client_id = Some(client_id);
        s
    }
    pub fn client_secret(self, client_secret: ClientSecret) -> Self {
        let mut s = self;
        s.client_secret = Some(client_secret);
        s
    }
    pub fn issuer_url(self, issuer_url: String) -> Self {
        let mut s = self;
        s.issuer_url = Some(issuer_url);
        s
    }
    pub fn auth_url(self, auth_url: AuthUrl) -> Self {
        let mut s = self;
        s.auth_url = Some(auth_url);
        s
    }
    pub fn token_url(self, token_url: TokenUrl) -> Self {
        let mut s = self;
        s.token_url = Some(token_url);
        s
    }
    pub fn userinfo_url(self, userinfo_url: String) -> Self {
        let mut s = self;
        s.userinfo_url = Some(userinfo_url);
        s
    }
    pub fn on_success_redirect(self, on_success_redirect: String) -> Self {
        let mut s = self;
        s.on_success_redirect = Some(on_success_redirect);
        s
    }
    pub fn on_failure_redirect(self, on_failure_redirect: String) -> Self {
        let mut s = self;
        s.on_failure_redirect = Some(on_failure_redirect);
        s
    }
    pub fn claims_audience(self, claims_audience: String) -> Self {
        let mut s = self;
        s.claims_audience = Some(claims_audience);
        s
    }
    pub fn claims_issuer(self, claims_issuer: String) -> Self {
        let mut s = self;
        s.claims_issuer = Some(claims_issuer);
        s
    }
    pub fn claims_expire_time(self, claims_expire_time: usize) -> Self {
        let mut s = self;
        s.claims_expire_time = Some(claims_expire_time);
        s
    }
    pub fn require_verified_email(self, require_verified_email: bool) -> Self {
        let mut s = self;
        s.require_verified_email = Some(require_verified_email);
        s
    }
    pub fn default_role(self, default_role: UserRole) -> Self {
        let mut s = self;
        s.default_role = Some(default_role);
        s
    }
    pub fn redirect_url(self, redirect_url: RedirectUrl) -> Self {
        let mut s = self;
        s.redirect_url = Some(redirect_url);
        s
    }
    pub fn allowed_roles(self, allowed_roles: &[String]) -> Self {
        let mut s = self;
        s.allowed_roles.extend(allowed_roles.iter().cloned());
        s
    }
    pub fn admin_roles(self, admin_roles: &[String]) -> Self {
        let mut s = self;
        s.admin_roles.extend(admin_roles.iter().cloned());
        s
    }
    pub fn callbacks(self, callback: OAuthCallbackFn) -> Self {
        let mut s = self;
        s.callbacks.push(callback);
        s
    }
    pub fn build(self) -> Result<ServiceGroup, Error> {
        let client_id = self.client_id.ok_or(Error::new(
            ErrorKind::InvalidInput,
            "OAuth client_id not set",
        ))?;
        let client_secret = self.client_secret.ok_or(Error::new(
            ErrorKind::InvalidInput,
            "OAuth client_secret not set",
        ))?;
        let issuer_url = self.issuer_url.ok_or(Error::new(
            ErrorKind::InvalidInput,
            "OAuth issuer_url not set",
        ))?;
        let auth_url = self.auth_url.ok_or(Error::new(
            ErrorKind::InvalidInput,
            "OAuth auth_url not set",
        ))?;
        let token_url = self.token_url.ok_or(Error::new(
            ErrorKind::InvalidInput,
            "OAuth token_url not set",
        ))?;
        let userinfo_url = self.userinfo_url.ok_or(Error::new(
            ErrorKind::InvalidInput,
            "OAuth userinfo_url not set",
        ))?;
        let redirect_url = self.redirect_url.ok_or(Error::new(
            ErrorKind::InvalidInput,
            "OAuth redirect_url not set",
        ))?;
        let config = Arc::new(OAuthConfig {
            client: BasicClient::new(
                client_id.clone(),
                Some(client_secret.clone()),
                auth_url.clone(),
                Some(token_url.clone()),
            )
            .set_redirect_uri(redirect_url),
            client_id,
            client_secret,
            issuer_url,
            auth_url,
            token_url,
            userinfo_url,
            on_success_redirect: self
                .on_success_redirect
                .unwrap_or_else(|| String::from("/")),
            on_failure_redirect: self
                .on_failure_redirect
                .unwrap_or_else(|| String::from("/")),
            claims_audience: self.claims_audience.unwrap_or_default(),
            claims_issuer: self.claims_issuer.unwrap_or_default(),
            claims_expire_time: self.claims_expire_time.unwrap_or(0),
            require_verified_email: self.require_verified_email.unwrap_or(true),
            default_role: self.default_role.unwrap_or(UserRole::None),
            allowed_roles: self.allowed_roles,
            admin_roles: self.admin_roles,
            callbacks: self.callbacks,
        });
        debug!("Configured keycloak oauth issuer={}", config.issuer_url);
        let login_service = ServiceBuilder::new("/keycloak/login")
            .name("keycloak_login")
            .handler(Arc::new(OAuthLoginHandler {
                config: config.clone(),
            }))
            .build();
        let auth_service = ServiceBuilder::new("/keycloak/auth")
            .name("keycloak_auth")
            .handler(Arc::new(OAuthAuthHandler {
                config: config.clone(),
            }))
            .build();
        Ok(ServiceGroup::default()
            .service(login_service)
            .service(auth_service))
    }
}

fn csv_env(var_name: &str) -> Vec<String> {
    env::var(var_name)
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(values: &[&str]) -> HashSet<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn default_role_does_not_bypass_role_allowlist() {
        let role = resolve_authorized_role(
            UserRole::User,
            &HashSet::new(),
            &set(&["nebula-admin"]),
            &HashSet::new(),
        );

        assert_eq!(role, UserRole::None);
    }

    #[test]
    fn admin_role_overrides_default_role() {
        let role = resolve_authorized_role(
            UserRole::User,
            &set(&["nebula-admin"]),
            &set(&["nebula-admin"]),
            &HashSet::new(),
        );

        assert_eq!(role, UserRole::Admin);
    }

    #[test]
    fn allowed_roles_still_gate_non_admin_access() {
        let denied_role = resolve_authorized_role(
            UserRole::User,
            &HashSet::new(),
            &set(&["nebula-admin"]),
            &set(&["nebula-user"]),
        );
        let allowed_role = resolve_authorized_role(
            UserRole::User,
            &set(&["nebula-user"]),
            &set(&["nebula-admin"]),
            &set(&["nebula-user"]),
        );

        assert_eq!(denied_role, UserRole::None);
        assert_eq!(allowed_role, UserRole::User);
    }

    #[test]
    fn allowed_roles_fall_back_to_user_when_default_role_is_none() {
        let allowed_role = resolve_authorized_role(
            UserRole::None,
            &set(&["nebula-user"]),
            &set(&["nebula-admin"]),
            &set(&["nebula-user"]),
        );

        assert_eq!(allowed_role, UserRole::User);
    }

    #[test]
    fn extracts_roles_from_access_token_payload() {
        let token = "eyJhbGciOiJub25lIn0.eyJyZWFsbV9hY2Nlc3MiOnsicm9sZXMiOlsibmVidWxhLWFkbWluIl19LCJyZXNvdXJjZV9hY2Nlc3MiOnsibmVidWxhLWNtcy1kZXYtcmVhbG0iOnsicm9sZXMiOlsibmVidWxhLXVzZXIiXX19fQ.";
        let roles = extract_roles_from_access_token(token).unwrap();

        assert!(roles.contains("nebula-admin"));
        assert!(roles.contains("nebula-user"));
    }

    #[test]
    fn unverified_email_is_not_trusted_for_subject_or_authorization() {
        let user_info = UserInfo {
            sub: "subject-id".to_string(),
            email: "person@example.com".to_string(),
            email_verified: false,
            preferred_username: String::new(),
            ..Default::default()
        };

        let trusted_email = trusted_userinfo_email(&user_info, true);
        assert!(trusted_email.is_none());
        assert_eq!(
            resolved_subject(&user_info, trusted_email.as_deref()),
            "subject-id"
        );
    }

    #[test]
    fn verified_email_is_available_for_authorization_when_present() {
        let user_info = UserInfo {
            sub: "subject-id".to_string(),
            email: "person@example.com".to_string(),
            email_verified: true,
            preferred_username: "nebula-user".to_string(),
            ..Default::default()
        };

        let trusted_email = trusted_userinfo_email(&user_info, true);
        assert_eq!(trusted_email.as_deref(), Some("person@example.com"));
        assert_eq!(
            resolved_subject(&user_info, trusted_email.as_deref()),
            "nebula-user"
        );
    }

    #[test]
    fn unverified_email_can_be_used_when_requirement_is_disabled() {
        let user_info = UserInfo {
            sub: "subject-id".to_string(),
            email: "person@example.com".to_string(),
            email_verified: false,
            preferred_username: String::new(),
            ..Default::default()
        };

        let trusted_email = trusted_userinfo_email(&user_info, false);
        assert_eq!(trusted_email.as_deref(), Some("person@example.com"));
        assert_eq!(
            resolved_subject(&user_info, trusted_email.as_deref()),
            "person@example.com"
        );
    }

    #[test]
    fn auth_request_defaults_missing_callback_fields() {
        let body: AuthRequest = serde_json::from_value(serde_json::json!({
            "state": "abc123"
        }))
        .expect("auth request should deserialize with defaults");

        assert_eq!(body.code, "");
        assert_eq!(body.state, "abc123");
        assert_eq!(body.error, "");
        assert_eq!(body.error_description, "");
    }
}
