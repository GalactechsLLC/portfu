use crate::auth::Claims;
use crate::services::{redirect_to_url, send_internal_error};
use crate::users::UserRole;
use http::HeaderValue;
use hyper::{header, StatusCode};
use oauth2::basic::BasicClient;
use oauth2::reqwest::async_http_client;
use oauth2::{
    AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, PkceCodeChallenge, RedirectUrl,
    Scope, TokenResponse, TokenUrl,
};
use octocrab::models::orgs::Organization;
use octocrab::models::Author;
use portfu::pfcore::service::{ServiceBuilder, ServiceGroup};
use portfu::pfcore::{FromRequest, Json, Query, ServiceData, ServiceHandler, ServiceType, State};
use portfu::prelude::async_trait;
use portfu::wrappers::sessions::Session;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
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
}

#[derive(Default, Clone, Deserialize)]
pub struct UserData {
    pub user_id: u64,
    pub email: String,
    pub org_ids: Vec<u64>,
    pub user_role: UserRole,
}

#[derive(Default, Clone, Deserialize)]
pub struct AuthRequest {
    code: String,
    state: String,
}

pub enum OAuthCallbackFn {
    OnSuccess(
        Pin<
            Box<
                dyn Fn(usize) -> Box<dyn Future<Output = Result<(), Error>>>
                    + Send
                    + Sync
                    + 'static,
            >,
        >,
    ),
    OnFailure(
        Pin<
            Box<
                dyn Fn(usize) -> Box<dyn Future<Output = Result<(), Error>>>
                    + Send
                    + Sync
                    + 'static,
            >,
        >,
    ),
}

pub struct OAuthLoginHandler {
    config: Arc<OAuthConfig>,
}

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
        // Create a PKCE code verifier and SHA-256 encode it as a code challenge.
        let (pkce_code_challenge, _pkce_code_verifier) = PkceCodeChallenge::new_random_sha256();
        // Generate the authorization URL to which we'll redirect the user.
        let client = &self.config.client;
        let mut auth_request = client
            .authorize_url(CsrfToken::new_random)
            // Set the desired scopes.
            .add_scope(Scope::new("read:user user:email read:org".to_string()))
            // Set the PKCE code challenge.
            .set_pkce_challenge(pkce_code_challenge);
        if let Some(redirect_params) = redirect_params {
            if let Ok(session) = State::<RwLock<Session>>::from_request(&mut data.request, "")
                .await
                .map(|q| q.inner())
            {
                session.write().await.data.insert(redirect_params);
            }
        }
        let (auth_url, _csrf_token) = client.url();
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
#[async_trait::async_trait]
impl ServiceHandler for OAuthAuthHandler {
    fn name(&self) -> &str {
        "auth"
    }
    async fn handle(
        &self,
        mut data: portfu::prelude::ServiceData,
    ) -> Result<ServiceData, (ServiceData, Error)> {
        let body: Option<AuthRequest> = match Json::from_request(&mut data.request, "").await {
            Ok(json) => json.inner(),
            Err(_) => None,
        };
        let body: AuthRequest = match body {
            None => match Query::<Option<AuthRequest>>::from_request(&mut data.request, "").await {
                Ok(v) => match v.inner() {
                    Some(v) => v,
                    None => {
                        return send_internal_error(data, "Failed to extract AuthRequest");
                    }
                },
                Err(e) => {
                    return send_internal_error(
                        data,
                        format!("Failed to extract Query as AuthRequest, {e:?}"),
                    );
                }
            },
            Some(v) => v,
        };
        let session = if let Some(session) = data.request.get_mut::<Arc<RwLock<Session>>>() {
            session
        } else {
            return send_internal_error(data, "Failed to Find Session to Auth".to_string());
        };
        let code = AuthorizationCode::new(body.code.clone());
        let _token_state = CsrfToken::new(body.state.clone());
        let client = &self.config.client;
        let token = if let Ok(token) = client
            .exchange_code(code)
            .request_async(async_http_client)
            .await
        {
            token
        } else {
            return redirect_to_url(data, self.config.on_failure_redirect.as_str());
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
            return redirect_to_url(data, self.config.on_failure_redirect.as_str());
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
            return redirect_to_url(data, self.config.on_failure_redirect.as_str());
        };
        let mut claims: Claims = session.read().await.data.get().cloned().unwrap_or(Claims {
            aud: self.config.claims_audience.clone(),
            exp: self.config.claims_expire_time.clone(), //30 * 60, //30 Minutes
            iat: OffsetDateTime::now_utc().unix_timestamp() as usize,
            iss: self.config.claims_issuer.clone(),
            nbf: OffsetDateTime::now_utc().unix_timestamp() as usize,
            sub: "".to_string(),
            eml: "".to_string(),
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
            claims.sub = user_info.id.to_string();
            claims.eml = user_info.email.unwrap_or_default();
        }
        session.write().await.data.insert(claims);
        if let Ok(session) = State::<RwLock<Session>>::from_request(&mut data.request, "")
            .await
            .map(|q| q.inner())
        {
            if let Some(redirect) = session
                .read()
                .await
                .data
                .remove::<OAuthLoginRedirectParams>()
            {
                return redirect_to_url(data, redirect.redirect_url.as_str());
            }
        }
        redirect_to_url(data, self.config.on_success_redirect.as_str())
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
    pub allowed_organizations: Vec<u64>,
    pub callbacks: Vec<OAuthCallbackFn>,
    pub allowed_users: Vec<u64>,
    pub admin_users: Vec<u64>,
}
impl OAuthLoginBuilder {
    pub fn from_env() -> Self {
        let oauthserver =
            env::var("OAUTH_SERVER").expect("Missing the OAUTH_SERVER environment variable.");
        OAuthLoginBuilder::new()
            .client_id(ClientId::new(
                env::var("OAUTH_CLIENT_ID")
                    .expect("Missing the OAUTH_CLIENT_ID environment variable."),
            ))
            .client_secret(ClientSecret::new(
                env::var("OAUTH_CLIENT_SECRET")
                    .expect("Missing the OAUTH_CLIENT_SECRET environment variable."),
            ))
            .oauthserver(oauthserver.clone())
            .auth_url(
                AuthUrl::new(format!("https://{}/oauth/authorize", oauthserver))
                    .expect("Invalid authorization endpoint URL"),
            )
            .on_success_redirect(
                env::var("OAUTH_SUCCESS_URL").unwrap_or_else(|_| String::from("/")),
            )
            .on_failure_redirect(
                env::var("OAUTH_FAILURE_URL").unwrap_or_else(|_| String::from("/")),
            )
            .token_url(
                TokenUrl::new(format!("https://{}/oauth/access_token", oauthserver))
                    .expect("Invalid token endpoint URL"),
            )
            .api_base_url(format!("https://{}/api/v4", oauthserver))
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
            .redirect_url(
                RedirectUrl::new(
                    env::var("OAUTH_REDIRECT_URL")
                        .expect("Missing the OAUTH_REDIRECT_URL environment variable."),
                )
                .expect("Invalid redirect URL"),
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
            allowed_users: self.allowed_users,
            admin_users: self.admin_users,
        });
        let login_service = ServiceBuilder::new("/github/login")
            .name("index")
            .handler(Arc::new(OAuthLoginHandler {
                config: config.clone(),
            }))
            .build();
        let auth_service = ServiceBuilder::new("/github/auth")
            .name("index")
            .handler(Arc::new(OAuthAuthHandler {
                config: config.clone(),
            }))
            .build();
        Ok(ServiceGroup::default()
            .service(login_service)
            .service(auth_service))
    }
}
