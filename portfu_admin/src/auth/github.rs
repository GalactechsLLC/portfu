use crate::services::{redirect_to_url, send_internal_error};
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
use portfu::filters::method::GET;
use portfu::pfcore::service::{ServiceBuilder, ServiceGroup};
use portfu::pfcore::{FromRequest, Json, ServiceData, ServiceHandler};
use portfu::prelude::{async_trait, Body};
use portfu::wrappers::sessions::Session;
use serde::Deserialize;
use std::env;
use std::io::{Error, ErrorKind};
use std::sync::Arc;

pub struct OAuthConfig {
    pub client: BasicClient,
    pub client_id: ClientId,
    pub client_secret: ClientSecret,
    pub oauthserver: String,
    pub auth_url: AuthUrl,
    pub token_url: TokenUrl,
    pub api_base_url: String,
    pub allowed_organizations: Vec<u64>,
    pub allowed_users: Vec<u64>,
    pub admin_users: Vec<u64>,
}

#[derive(Default, Clone, Deserialize)]
pub enum UserLevel {
    User,
    #[default]
    Admin,
}

#[derive(Default, Clone, Deserialize)]
pub struct UserData {
    pub user_id: Vec<u64>,
    pub org_id: Vec<u64>,
    pub user_level: UserLevel,
}

#[derive(Default, Clone, Deserialize)]
pub struct AuthRequest {
    code: String,
    state: String,
}

pub struct OAuthLoginHandler {
    config: Arc<OAuthConfig>,
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
        // Create a PKCE code verifier and SHA-256 encode it as a code challenge.
        let (pkce_code_challenge, _pkce_code_verifier) = PkceCodeChallenge::new_random_sha256();
        // Generate the authorization URL to which we'll redirect the user.
        let client = &self.config.client;
        let (auth_url, _csrf_token) = client
            .authorize_url(CsrfToken::new_random)
            // Set the desired scopes.
            .add_scope(Scope::new("read:user user:email read:org".to_string()))
            // Set the PKCE code challenge.
            .set_pkce_challenge(pkce_code_challenge)
            .url();
        *data.response.status_mut() = StatusCode::FOUND;
        data.response.headers_mut().insert(
            header::LOCATION,
            HeaderValue::from_str(auth_url.as_str()).unwrap_or(HeaderValue::from_static("/")),
        );
        Ok(data)
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
        let mut user_data: UserData = if let Some(session) = data.request.get_mut::<Session>() {
            session.data.remove().unwrap_or(UserData {
                user_id: vec![],
                org_id: vec![],
                user_level: UserLevel::User,
            })
        } else {
            return send_internal_error(data, "Failed to Find Session to Auth".to_string());
        };
        let body: Json<AuthRequest> = match Body::from_request(&mut data.request, "").await {
            Ok(v) => v.inner(),
            Err(e) => {
                return send_internal_error(
                    data,
                    format!("Failed to extract Body as AuthRequest, {e:?}"),
                );
            }
        };
        let body: AuthRequest = body.inner();
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
            return redirect_to_url(data, "/".to_string());
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
            return redirect_to_url(data, "/".to_string());
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
            let text = org_info.text().await.unwrap_or_default();
            serde_json::from_str(&text).ok()
        } else {
            return redirect_to_url(data, "/".to_string());
        };
        if let Some(org_list) = &org_info {
            for org in org_list {
                if self.config.allowed_organizations.contains(&org.id.0) {
                    user_data.user_level = UserLevel::User;
                    break;
                }
            }
        }
        if let Some(user_info) = &user_info {
            if self.config.admin_users.contains(&user_info.id.0) {
                user_data.user_level = UserLevel::Admin;
            } else if self.config.allowed_users.contains(&user_info.id.0) {
                user_data.user_level = UserLevel::User;
            }
        }
        data.request.insert(user_data);
        redirect_to_url(data, "/admin".to_string())
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
    pub allowed_organizations: Vec<u64>,
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
            .oauthserver(
                env::var("OAUTH_SERVER").expect("Missing the OAUTH_SERVER environment variable."),
            )
            .auth_url(
                AuthUrl::new(format!("https://{}/oauth/authorize", oauthserver))
                    .expect("Invalid authorization endpoint URL"),
            )
            .token_url(
                TokenUrl::new(format!("https://{}/oauth/access_token", oauthserver))
                    .expect("Invalid token endpoint URL"),
            )
            .api_base_url(format!("https://{}/api/v4", oauthserver))
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
            allowed_users: self.allowed_users,
            admin_users: self.admin_users,
        });
        let login_service = ServiceBuilder::new("/github/login")
            .name("index")
            .filter(GET.clone())
            .handler(Arc::new(OAuthLoginHandler {
                config: config.clone(),
            }))
            .build();
        let auth_service = ServiceBuilder::new("/github/auth")
            .name("index")
            .filter(GET.clone())
            .handler(Arc::new(OAuthAuthHandler {
                config: config.clone(),
            }))
            .build();
        Ok(ServiceGroup::default()
            .service(login_service)
            .service(auth_service))
    }
}
