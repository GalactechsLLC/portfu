use crate::auth::Claims;
use crate::services::{redirect_to_url, send_internal_error};
use crate::users::UserRole;
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
    pub default_role: UserRole,
    pub allowed_roles: Vec<String>,
    pub admin_roles: Vec<String>,
    pub allowed_users: Vec<String>,
    pub admin_users: Vec<String>,
    pub callbacks: Vec<OAuthCallbackFn>,
}

#[derive(Default, Clone, Deserialize)]
pub struct AuthRequest {
    code: String,
    state: String,
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
    preferred_username: String,
    #[serde(default)]
    realm_access: Option<RoleCollection>,
    #[serde(default)]
    resource_access: HashMap<String, RoleCollection>,
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
            Some(q)
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
        let token_val = format!("Bearer {}", token.access_token().secret());
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
        claims.uid = user_info.sub.clone();
        claims.eml = user_info.email.clone();
        claims.sub = if !user_info.preferred_username.is_empty() {
            user_info.preferred_username.clone()
        } else if !user_info.email.is_empty() {
            user_info.email.clone()
        } else {
            user_info.sub.clone()
        };
        let mut extracted_roles: HashSet<String> = HashSet::new();
        if let Some(realm_access) = &user_info.realm_access {
            for role in &realm_access.roles {
                if !role.is_empty() {
                    extracted_roles.insert(role.to_ascii_lowercase());
                }
            }
        }
        for resource in user_info.resource_access.values() {
            for role in &resource.roles {
                if !role.is_empty() {
                    extracted_roles.insert(role.to_ascii_lowercase());
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
        let has_role_constraints = !(admin_roles.is_empty() && allowed_roles.is_empty());
        let has_user_constraints =
            !(self.config.admin_users.is_empty() && self.config.allowed_users.is_empty());
        let mut authorized = !(has_role_constraints || has_user_constraints);
        if !admin_roles.is_empty()
            && extracted_roles
                .iter()
                .any(|role| admin_roles.contains(role))
        {
            claims.rol = UserRole::Admin;
            authorized = true;
        } else if !allowed_roles.is_empty()
            && extracted_roles
                .iter()
                .any(|role| allowed_roles.contains(role))
        {
            claims.rol = UserRole::User;
            authorized = true;
        }
        let user_identifiers = vec![user_info.sub, user_info.email, user_info.preferred_username];
        if !self.config.admin_users.is_empty()
            && user_identifiers
                .iter()
                .filter(|v| !v.is_empty())
                .any(|identifier| {
                    self.config
                        .admin_users
                        .iter()
                        .any(|candidate| candidate.eq_ignore_ascii_case(identifier))
                })
        {
            claims.rol = UserRole::Admin;
            authorized = true;
        } else if !self.config.allowed_users.is_empty()
            && user_identifiers
                .iter()
                .filter(|v| !v.is_empty())
                .any(|identifier| {
                    self.config
                        .allowed_users
                        .iter()
                        .any(|candidate| candidate.eq_ignore_ascii_case(identifier))
                })
        {
            claims.rol = UserRole::User;
            authorized = true;
        }
        if !authorized {
            claims.rol = UserRole::None;
        }
        session.write().await.data.insert(claims.clone());
        info!("Running OAuth success handlers");
        let maybe_redirect = session
            .write()
            .await
            .data
            .remove::<OAuthLoginRedirectParams>();
        let url = if let Some(redirect) = maybe_redirect {
            redirect.redirect_url.clone()
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
    pub default_role: Option<UserRole>,
    pub callbacks: Vec<OAuthCallbackFn>,
    pub allowed_roles: Vec<String>,
    pub admin_roles: Vec<String>,
    pub allowed_users: Vec<String>,
    pub admin_users: Vec<String>,
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
            .unwrap_or(UserRole::User);
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
                .default_role(default_role)
                .allowed_roles(&csv_env("KEYCLOAK_ALLOWED_ROLES"))
                .admin_roles(&csv_env("KEYCLOAK_ADMIN_ROLES"))
                .allowed_users(&csv_env("KEYCLOAK_ALLOWED_USERS"))
                .admin_users(&csv_env("KEYCLOAK_ADMIN_USERS")),
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
    pub fn allowed_users(self, allowed_users: &[String]) -> Self {
        let mut s = self;
        s.allowed_users.extend(allowed_users.iter().cloned());
        s
    }
    pub fn admin_users(self, admin_users: &[String]) -> Self {
        let mut s = self;
        s.admin_users.extend(admin_users.iter().cloned());
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
            default_role: self.default_role.unwrap_or(UserRole::User),
            allowed_roles: self.allowed_roles,
            admin_roles: self.admin_roles,
            allowed_users: self.allowed_users,
            admin_users: self.admin_users,
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
