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
use octocrab::models::orgs::Organization;
use octocrab::models::Author;
use portfu::pfcore::services::builder::ServiceBuilder;
use portfu::pfcore::services::group::ServiceGroup;
use portfu::pfcore::{FromRequest, Json, Query, ServiceData, ServiceHandler, ServiceType};
use portfu::prelude::async_trait;
use portfu::wrappers::sessions::Session;
use serde::{Deserialize, Serialize};
use std::env;
use std::future::Future;
use std::io::{Error, ErrorKind};
use std::num::ParseIntError;
use std::pin::Pin;
use std::sync::Arc;
use time::OffsetDateTime;
use tokio::sync::RwLock;

pub struct OAuthConfig {
    pub client: BasicClient,
    pub client_id: ClientId,
    pub client_secret: ClientSecret,
    pub oauthserver: String,
    pub auth_url: AuthUrl,
    pub token_url: TokenUrl,
    pub api_base_url: String,
    pub on_success_redirect: String,
    pub on_failure_redirect: String,
    pub claims_audience: String,
    pub claims_issuer: String,
    pub claims_expire_time: usize,
    pub allowed_organizations: Vec<u64>,
    pub allowed_users: Vec<u64>,
    pub admin_users: Vec<u64>,
    pub callbacks: Vec<OAuthCallbackFn>,
}

#[derive(Default, Clone, Deserialize)]
pub struct UserData {
    pub user_id: u64,
    pub email: String,
    pub org_ids: Vec<u64>,
    pub user_role: UserRole,
}

#[derive(Default, Clone, Deserialize)]
struct EmailEntry {
    email: String,
    primary: bool,
    verified: bool,
}

#[derive(Default, Clone, Deserialize)]
pub struct AuthRequest {
    code: String,
    state: String,
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
        //Check if there is a current_page query
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
            warn!("Failed to Find session to auth");
            return Ok(send_internal_error(data, "Failed to Find Session to Auth"));
        };
        // Create a PKCE code verifier and SHA-256 encode it as a code challenge.
        let (pkce_code_challenge, pkce_code_verifier) = PkceCodeChallenge::new_random_sha256();
        session
            .write()
            .await
            .data
            .insert(Verifier(pkce_code_verifier.secret().to_string()));
        // Generate the authorization URL to which we'll redirect the user.
        let client = &self.config.client;
        let auth_request = client
            .authorize_url(CsrfToken::new_random)
            .add_scope(Scope::new("read:user".into()))
            .add_scope(Scope::new("user:email".into()))
            .add_scope(Scope::new("read:org".into()))
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
    pub async fn handle_error(
        &self,
        mut data: ServiceData,
        error: &str,
    ) -> Result<ServiceData, (ServiceData, Error)> {
        for callback in &self.config.callbacks {
            if let OAuthCallbackFn::OnFailure(err_fn) = callback {
                match (*err_fn)(data).await? {
                    CallbackResult::Continue(d) => {
                        data = d;
                    }
                    CallbackResult::Return(data) => {
                        return Ok(data);
                    }
                }
            }
        }
        Ok(send_internal_error(data, error))
    }
    pub async fn handle_failure(
        &self,
        mut data: ServiceData,
        url: &str,
    ) -> Result<ServiceData, (ServiceData, Error)> {
        for callback in &self.config.callbacks {
            if let OAuthCallbackFn::OnFailure(err_fn) = callback {
                match (*err_fn)(data).await? {
                    CallbackResult::Continue(d) => {
                        data = d;
                    }
                    CallbackResult::Return(data) => {
                        return Ok(data);
                    }
                }
            }
        }
        Ok(redirect_to_url(data, url))
    }
    pub async fn handle_success(
        &self,
        mut data: ServiceData,
        claims: Claims,
        url: &str,
    ) -> Result<ServiceData, (ServiceData, Error)> {
        for callback in &self.config.callbacks {
            if let OAuthCallbackFn::OnSuccess(callback) = callback {
                match callback(claims.clone(), data).await? {
                    CallbackResult::Continue(d) => {
                        data = d;
                    }
                    CallbackResult::Return(data) => {
                        return Ok(data);
                    }
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
            warn!("Failed to Find session to auth");
            return self
                .handle_error(data, "Failed to Find Session to Auth")
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
                        warn!("Failed to Extract Request");
                        return self
                            .handle_error(data, "Failed to extract AuthRequest")
                            .await;
                    }
                },
                Err(e) => {
                    warn!("Failed to Extract Query");
                    return self
                        .handle_error(
                            data,
                            &format!("Failed to extract Query as AuthRequest, {e:?}"),
                        )
                        .await;
                }
            },
            Some(v) => v,
        };
        let existing_token = if let Some(session) = session.read().await.data.get::<CsrfToken>() {
            session.clone()
        } else {
            warn!("Failed to Find Csrf Token");
            return self.handle_error(data, "Failed to Find Csrf Token").await;
        };
        let token_state = CsrfToken::new(body.state.clone());
        if existing_token.secret() != token_state.secret() {
            warn!("Invalid Csrf Token");
            return self.handle_error(data, "Invalid Csrf Token").await;
        }
        let verifier = if let Some(verifier) = session.read().await.data.get::<Verifier>() {
            PkceCodeVerifier::new(verifier.0.clone())
        } else {
            warn!("Failed to Find Verifier");
            return self.handle_error(data, "Failed to Find Verifier").await;
        };
        let code = AuthorizationCode::new(body.code.clone());
        let client = &self.config.client;
        let token = match client
            .exchange_code(code)
            .set_pkce_verifier(verifier)
            .request_async(async_http_client)
            .await
        {
            Ok(token) => token,
            Err(e) => {
                warn!("Failed to Get Auth Token: {e:?}");
                return self
                    .handle_failure(data, self.config.on_failure_redirect.as_str())
                    .await;
            }
        };
        let token_val = format!("Bearer {}", token.access_token().secret());
        let client = reqwest::Client::builder().build().unwrap();
        let user_info: Option<Author> = if let Ok(user_info) = client
            .get("https://api.github.com/user")
            .header("Authorization", &token_val)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "portfu-login-service")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
        {
            user_info.json().await.ok()
        } else {
            warn!("Failed to Load User Info");
            return self
                .handle_failure(data, self.config.on_failure_redirect.as_str())
                .await;
        };
        let org_info: Option<Vec<Organization>> = if let Ok(org_info) = client
            .get("https://api.github.com/user/orgs")
            .header("Authorization", &token_val)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "portfu-login-service")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
        {
            org_info.json().await.ok()
        } else {
            warn!("Failed to Load Org Info");
            return self
                .handle_failure(data, self.config.on_failure_redirect.as_str())
                .await;
        };
        let mut claims: Claims = session.read().await.data.get().cloned().unwrap_or(Claims {
            aud: self.config.claims_audience.clone(),
            exp: OffsetDateTime::now_utc().unix_timestamp() as usize
                + self.config.claims_expire_time, //30 * 60, //30 Minutes
            iat: OffsetDateTime::now_utc().unix_timestamp() as usize,
            iss: self.config.claims_issuer.clone(),
            nbf: OffsetDateTime::now_utc().unix_timestamp() as usize,
            sub: "".to_string(),
            eml: "".to_string(),
            uid: "".to_string(),
            rol: UserRole::None,
            org: vec![],
        });
        if let Some(org_list) = &org_info {
            for org in org_list {
                claims.org.push(org.id.0);
                if self.config.allowed_organizations.contains(&org.id.0) {
                    claims.rol = UserRole::User;
                }
            }
        }
        if let Some(user_info) = user_info {
            if self.config.admin_users.contains(&user_info.id.0) {
                claims.rol = UserRole::Admin;
            } else if self.config.allowed_users.contains(&user_info.id.0) {
                claims.rol = UserRole::User;
            }
            if let Ok(emails) = client
                .get("https://api.github.com/user/emails")
                .header("Authorization", &token_val)
                .header("Accept", "application/vnd.github+json")
                .header("User-Agent", "portfu-login-service")
                .header("X-GitHub-Api-Version", "2022-11-28")
                .send()
                .await
            {
                match emails.json().await {
                    Ok(_emails) => {
                        let emails: Vec<EmailEntry> = _emails;
                        let mut email = None;
                        for entry in emails {
                            if entry.primary && entry.verified {
                                debug!("New Verified Primary Found");
                                email = Some(entry);
                                break;
                            } else if entry.verified
                                && (email.is_none()
                                    || (email.is_some()
                                        && !email
                                            .as_ref()
                                            .expect("Just Checked Email is Some")
                                            .verified))
                            {
                                debug!("Verified Found");
                                email = Some(entry);
                            } else if entry.primary
                                && (email.is_none()
                                    || (email.is_some()
                                        && !email
                                            .as_ref()
                                            .expect("Just Checked Email is Some")
                                            .verified))
                            {
                                debug!("Unverified Primary Found");
                                email = Some(entry);
                            } else if email.is_none() {
                                email = Some(entry);
                            } else {
                                continue;
                            }
                        }
                        claims.eml = email.map(|v| v.email).unwrap_or_default();
                    }
                    Err(e) => {
                        warn!("Failed to Parse Emails Response: {e:?}");
                        return self
                            .handle_failure(data, self.config.on_failure_redirect.as_str())
                            .await;
                    }
                }
            } else {
                warn!("Failed to Load User Emails");
                claims.eml = user_info.email.unwrap_or_default();
            }
            claims.sub = user_info.login;
            claims.uid = user_info.id.to_string();
        }
        session.write().await.data.insert(claims.clone());
        info!("Running OAuth Success handles");
        if let Some(redirect) = session
            .write()
            .await
            .data
            .remove::<OAuthLoginRedirectParams>()
        {
            return self
                .handle_success(data, claims, redirect.redirect_url.as_str())
                .await;
        }
        self.handle_success(data, claims, self.config.on_success_redirect.as_str())
            .await
    }

    fn service_type(&self) -> ServiceType {
        ServiceType::API
    }
}

#[derive(Default)]
pub struct OAuthLoginBuilder {
    pub client_id: Option<ClientId>,
    pub client_secret: Option<ClientSecret>,
    pub oauthserver: Option<String>,
    pub auth_url: Option<AuthUrl>,
    pub token_url: Option<TokenUrl>,
    pub api_base_url: Option<String>,
    pub redirect_url: Option<RedirectUrl>,
    pub on_success_redirect: Option<String>,
    pub on_failure_redirect: Option<String>,
    pub claims_audience: Option<String>,
    pub claims_issuer: Option<String>,
    pub claims_expire_time: Option<usize>,
    pub allowed_organizations: Vec<u64>,
    pub callbacks: Vec<OAuthCallbackFn>,
    pub allowed_users: Vec<u64>,
    pub admin_users: Vec<u64>,
}
impl OAuthLoginBuilder {
    pub fn from_env() -> Option<Self> {
        let oauthserver = match env::var("OAUTH_SERVER") {
            Ok(server) => server,
            Err(e) => {
                warn!("Failed to load OAUTH_SERVER: {}", e);
                return None;
            }
        };
        let client_id = match env::var("OAUTH_CLIENT_ID") {
            Ok(s) => ClientId::new(s),
            Err(e) => {
                warn!("Failed to load OAUTH_CLIENT_ID: {}", e);
                return None;
            }
        };
        let client_secret = match env::var("OAUTH_CLIENT_SECRET") {
            Ok(s) => ClientSecret::new(s),
            Err(e) => {
                warn!("Failed to load OAUTH_CLIENT_SECRET: {}", e);
                return None;
            }
        };
        let auth_url = match AuthUrl::new(format!("https://{oauthserver}/oauth/authorize")) {
            Ok(u) => u,
            Err(e) => {
                warn!("Failed to parse AuthUrl: {}", e);
                return None;
            }
        };
        let token_url = match TokenUrl::new(format!("https://{oauthserver}/oauth/access_token")) {
            Ok(u) => u,
            Err(e) => {
                warn!("Failed to parse TokenUrl: {}", e);
                return None;
            }
        };
        let redirect_str = match env::var("OAUTH_REDIRECT_URL") {
            Ok(server) => server,
            Err(e) => {
                warn!("Failed to load OAUTH_REDIRECT_URL: {}", e);
                return None;
            }
        };
        let redirect_url = match RedirectUrl::new(redirect_str) {
            Ok(u) => u,
            Err(e) => {
                warn!("Failed to parse RedirectUrl: {}", e);
                return None;
            }
        };
        Some(
            OAuthLoginBuilder::new()
                .client_id(client_id)
                .client_secret(client_secret)
                .oauthserver(oauthserver.clone())
                .auth_url(auth_url)
                .on_success_redirect(
                    env::var("OAUTH_SUCCESS_URL").unwrap_or_else(|_| String::from("/")),
                )
                .on_failure_redirect(
                    env::var("OAUTH_FAILURE_URL").unwrap_or_else(|_| String::from("/")),
                )
                .claims_issuer(
                    env::var("OAUTH_ISSUER").unwrap_or_else(|_| String::from("localhost")),
                )
                .claims_audience(
                    env::var("OAUTH_AUDIENCE").unwrap_or_else(|_| String::from("localhost")),
                )
                .claims_expire_time(
                    env::var("OAUTH_EXPIRE_TIME")
                        .map(|s| s.parse().unwrap_or(30usize * 60usize))
                        .unwrap_or(30usize * 60usize),
                )
                .token_url(token_url)
                .api_base_url(format!("https://{oauthserver}/api/v4"))
                .allowed_organizations(
                    &env::var("OAUTH_ORGANIZATIONS")
                        .unwrap_or_default()
                        .split(',')
                        .try_fold(vec![], |mut a, v| {
                            a.push(v.parse()?);
                            Ok::<Vec<u64>, ParseIntError>(a)
                        })
                        .unwrap_or_default(),
                )
                .allowed_users(
                    &env::var("OAUTH_USERS")
                        .unwrap_or_default()
                        .split(',')
                        .try_fold(vec![], |mut a, v| {
                            a.push(v.parse()?);
                            Ok::<Vec<u64>, ParseIntError>(a)
                        })
                        .unwrap_or_default(),
                )
                .admin_users(
                    &env::var("OAUTH_ADMINS")
                        .unwrap_or_default()
                        .split(',')
                        .try_fold(vec![], |mut a, v| {
                            a.push(v.parse()?);
                            Ok::<Vec<u64>, ParseIntError>(a)
                        })
                        .unwrap_or_default(),
                )
                .redirect_url(redirect_url),
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
    pub fn oauthserver(self, oauthserver: String) -> Self {
        let mut s = self;
        s.oauthserver = Some(oauthserver);
        s
    }
    pub fn auth_url(self, auth_url: AuthUrl) -> Self {
        let mut s = self;
        s.auth_url = Some(auth_url);
        s
    }
    pub fn on_success_redirect(self, on_success_redirect: String) -> Self {
        let mut s = self;
        s.on_success_redirect = Some(on_success_redirect);
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
    pub fn on_failure_redirect(self, on_failure_redirect: String) -> Self {
        let mut s = self;
        s.on_failure_redirect = Some(on_failure_redirect);
        s
    }
    pub fn token_url(self, token_url: TokenUrl) -> Self {
        let mut s = self;
        s.token_url = Some(token_url);
        s
    }
    pub fn api_base_url(self, api_base_url: String) -> Self {
        let mut s = self;
        s.api_base_url = Some(api_base_url);
        s
    }
    pub fn redirect_url(self, redirect_url: RedirectUrl) -> Self {
        let mut s = self;
        s.redirect_url = Some(redirect_url);
        s
    }
    pub fn allowed_organizations(self, allowed_organizations: &[u64]) -> Self {
        let mut s = self;
        s.allowed_organizations.extend(allowed_organizations);
        s
    }
    pub fn allowed_users(self, allowed_users: &[u64]) -> Self {
        let mut s = self;
        s.allowed_users.extend(allowed_users);
        s
    }
    pub fn admin_users(self, admin_users: &[u64]) -> Self {
        let mut s = self;
        s.admin_users.extend(admin_users);
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
        let oauthserver = self.oauthserver.ok_or(Error::new(
            ErrorKind::InvalidInput,
            "OAuth oauthserver not set",
        ))?;
        let auth_url = self.auth_url.ok_or(Error::new(
            ErrorKind::InvalidInput,
            "OAuth auth_url not set",
        ))?;
        let token_url = self.token_url.ok_or(Error::new(
            ErrorKind::InvalidInput,
            "OAuth token_url not set",
        ))?;
        let api_base_url = self.api_base_url.ok_or(Error::new(
            ErrorKind::InvalidInput,
            "OAuth api_base_url not set",
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
            oauthserver,
            auth_url,
            token_url,
            api_base_url,
            allowed_organizations: self.allowed_organizations,
            on_success_redirect: self
                .on_success_redirect
                .unwrap_or_else(|| String::from("/")),
            on_failure_redirect: self
                .on_failure_redirect
                .unwrap_or_else(|| String::from("/")),
            claims_audience: self.claims_audience.unwrap_or_default(),
            claims_issuer: self.claims_issuer.unwrap_or_default(),
            allowed_users: self.allowed_users,
            admin_users: self.admin_users,
            callbacks: self.callbacks,
            claims_expire_time: self.claims_expire_time.unwrap_or(0),
        });
        let login_service = ServiceBuilder::new("/github/login")
            .name("github_login")
            .handler(Arc::new(OAuthLoginHandler {
                config: config.clone(),
            }))
            .build();
        let auth_service = ServiceBuilder::new("/github/auth")
            .name("github_auth")
            .handler(Arc::new(OAuthAuthHandler {
                config: config.clone(),
            }))
            .build();
        Ok(ServiceGroup::default()
            .service(login_service)
            .service(auth_service))
    }
}
