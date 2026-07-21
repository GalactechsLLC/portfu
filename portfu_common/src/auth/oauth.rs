use crate::error::PortfuError;
use crate::server::builder::ServerBuilder;
use crate::service::builder::ServiceBuilder;
use crate::service::request::{FromRequest, Query, Request};
use crate::service::response::Response;
use crate::service::traits::Service;
use crate::wrappers::sessions::Session;
use crate::wrappers::sessions::SessionManager;
use http::header::LOCATION;
use http::{HeaderValue, StatusCode};
use oauth2::basic::BasicClient;
use oauth2::reqwest::async_http_client;
use oauth2::{
    AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, Scope, TokenResponse, TokenUrl,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::env;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::RwLock;

#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum OAUTH {
    KEYCLOAK,
    GITHUB,
    CUSTOM,
}

#[derive(Clone, Debug)]
struct SessionCsrfToken(String);

#[derive(Clone, Debug)]
struct SessionPkceVerifier(String);

#[derive(Clone, Debug)]
pub struct SessionOAuthToken(pub OAuthToken);

#[derive(Clone, Debug)]
pub struct SessionOAuthIdentity(pub OAuthIdentity);

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OAuthCallbackQuery {
    pub code: String,
    pub state: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OAuthToken {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub token_type: String,
    pub expires_in_seconds: Option<u64>,
    pub scopes: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct OAuthIdentity {
    pub provider: OAUTH,
    pub subject: String,
    pub username: Option<String>,
    pub email: Option<String>,
    pub role: Option<String>,
    pub scopes: Vec<String>,
    pub raw: Value,
}

#[derive(Clone, Debug, Default)]
pub struct OAuthConfig {
    pub client_id: String,
    pub client_secret: String,
    pub auth_url: String,
    pub token_url: String,
    pub redirect_url: String,
}

impl OAuthConfig {
    pub fn from_env(prefix: &str) -> Result<Self, PortfuError> {
        let prefix = prefix.trim().to_ascii_uppercase();
        let required = |name: &str| -> Result<String, PortfuError> {
            env::var(format!("{prefix}_{name}")).map_err(|_| {
                PortfuError::Parsing(format!("Missing required oauth env var: {prefix}_{name}"))
            })
        };
        Ok(Self {
            client_id: required("CLIENT_ID")?,
            client_secret: required("CLIENT_SECRET")?,
            auth_url: required("AUTH_URL")?,
            token_url: required("TOKEN_URL")?,
            redirect_url: required("REDIRECT_URL")?,
        })
    }
}

#[derive(Clone)]
pub struct OAuthRouteConfig {
    pub provider: OAUTH,
    pub config: OAuthConfig,
    pub scopes: Vec<String>,
    pub login_path: String,
    pub callback_path: String,
    pub success_redirect: Option<String>,
    pub failure_redirect: Option<String>,
    pub policy: OAuthProviderPolicy,
}

impl OAuthRouteConfig {
    pub fn new(provider: OAUTH) -> Self {
        Self {
            provider,
            config: OAuthConfig::default(),
            scopes: default_scopes(provider),
            login_path: "/oauth/login".to_string(),
            callback_path: "/oauth/callback".to_string(),
            success_redirect: None,
            failure_redirect: None,
            policy: OAuthProviderPolicy::default(),
        }
    }
}

#[derive(Clone, Default)]
pub struct OAuthProviderPolicy {
    pub userinfo_url: Option<String>,
    pub api_base_url: Option<String>,
    pub allowed_users: Vec<String>,
    pub admin_users: Vec<String>,
    pub allowed_roles: Vec<String>,
    pub admin_roles: Vec<String>,
    pub allowed_organizations: Vec<String>,
    pub default_role: Option<String>,
    pub handler: Option<Arc<dyn OAuthPolicyHandler>>,
}

#[derive(Clone, Debug)]
pub struct OAuthPolicyContext {
    pub provider: OAUTH,
    pub token: OAuthToken,
    pub identity: OAuthIdentity,
    pub userinfo: Value,
}

#[derive(Clone, Debug)]
pub struct OAuthPolicyDecision {
    pub allow: bool,
    pub identity: OAuthIdentity,
    pub message: Option<String>,
}

impl OAuthPolicyDecision {
    pub fn allow(identity: OAuthIdentity) -> Self {
        Self {
            allow: true,
            identity,
            message: None,
        }
    }

    pub fn deny(identity: OAuthIdentity, message: impl Into<String>) -> Self {
        Self {
            allow: false,
            identity,
            message: Some(message.into()),
        }
    }
}

pub trait OAuthPolicyHandler: Send + Sync {
    fn evaluate(
        &self,
        context: OAuthPolicyContext,
    ) -> Pin<Box<dyn Future<Output = Result<OAuthPolicyDecision, PortfuError>> + Send + Sync>>;
}

impl<F, Fut> OAuthPolicyHandler for F
where
    F: Fn(OAuthPolicyContext) -> Fut + Send + Sync,
    Fut: Future<Output = Result<OAuthPolicyDecision, PortfuError>> + Send + Sync + 'static,
{
    fn evaluate(
        &self,
        context: OAuthPolicyContext,
    ) -> Pin<Box<dyn Future<Output = Result<OAuthPolicyDecision, PortfuError>> + Send + Sync>> {
        Box::pin(self(context))
    }
}

pub struct OAuthServerBuilder {
    builder: ServerBuilder,
    config: OAuthRouteConfig,
    session_manager: Option<SessionManager>,
}

impl OAuthServerBuilder {
    pub fn client_id<S: Into<String>>(mut self, client_id: S) -> Self {
        self.config.config.client_id = client_id.into();
        self
    }

    pub fn client_secret<S: Into<String>>(mut self, client_secret: S) -> Self {
        self.config.config.client_secret = client_secret.into();
        self
    }

    pub fn auth_url<S: Into<String>>(mut self, auth_url: S) -> Self {
        self.config.config.auth_url = auth_url.into();
        self
    }

    pub fn token_url<S: Into<String>>(mut self, token_url: S) -> Self {
        self.config.config.token_url = token_url.into();
        self
    }

    pub fn redirect_url<S: Into<String>>(mut self, redirect_url: S) -> Self {
        self.config.config.redirect_url = redirect_url.into();
        self
    }

    pub fn config(mut self, config: OAuthConfig) -> Self {
        self.config.config = config;
        self
    }

    pub fn from_env(mut self, prefix: &str) -> Result<Self, PortfuError> {
        self.config.config = OAuthConfig::from_env(prefix)?;
        Ok(self)
    }

    pub fn scope<S: Into<String>>(mut self, scope: S) -> Self {
        self.config.scopes.push(scope.into());
        self
    }

    pub fn scopes<I, S>(mut self, scopes: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.config.scopes = scopes.into_iter().map(Into::into).collect();
        self
    }

    pub fn login_path<S: Into<String>>(mut self, path: S) -> Self {
        self.config.login_path = path.into();
        self
    }

    pub fn callback_path<S: Into<String>>(mut self, path: S) -> Self {
        self.config.callback_path = path.into();
        self
    }

    pub fn success_redirect<S: Into<String>>(mut self, path: S) -> Self {
        self.config.success_redirect = Some(path.into());
        self
    }

    pub fn failure_redirect<S: Into<String>>(mut self, path: S) -> Self {
        self.config.failure_redirect = Some(path.into());
        self
    }

    pub fn userinfo_url<S: Into<String>>(mut self, url: S) -> Self {
        self.config.policy.userinfo_url = Some(url.into());
        self
    }

    pub fn api_base_url<S: Into<String>>(mut self, url: S) -> Self {
        self.config.policy.api_base_url = Some(url.into());
        self
    }

    pub fn allowed_user<S: Into<String>>(mut self, user: S) -> Self {
        self.config.policy.allowed_users.push(user.into());
        self
    }

    pub fn admin_user<S: Into<String>>(mut self, user: S) -> Self {
        self.config.policy.admin_users.push(user.into());
        self
    }

    pub fn allowed_role<S: Into<String>>(mut self, role: S) -> Self {
        self.config.policy.allowed_roles.push(role.into());
        self
    }

    pub fn admin_role<S: Into<String>>(mut self, role: S) -> Self {
        self.config.policy.admin_roles.push(role.into());
        self
    }

    pub fn allowed_organization<S: Into<String>>(mut self, organization: S) -> Self {
        self.config
            .policy
            .allowed_organizations
            .push(organization.into());
        self
    }

    pub fn default_role<S: Into<String>>(mut self, role: S) -> Self {
        self.config.policy.default_role = Some(role.into());
        self
    }

    pub fn policy<F, Fut>(mut self, handler: F) -> Self
    where
        F: Fn(OAuthPolicyContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<OAuthPolicyDecision, PortfuError>> + Send + Sync + 'static,
    {
        self.config.policy.handler = Some(Arc::new(handler));
        self
    }

    pub fn session_manager(mut self, manager: SessionManager) -> Self {
        self.session_manager = Some(manager);
        self
    }

    pub fn finish_oauth(self) -> ServerBuilder {
        let login = OAuthLoginService {
            config: self.config.clone(),
        };
        let callback = OAuthCallbackService {
            config: self.config.clone(),
        };
        let builder = self
            .builder
            .service(
                ServiceBuilder::new(self.config.login_path.as_str())
                    .name("oauth_login")
                    .filter(crate::router::filter::method::GET.clone())
                    .handler(Arc::new(login))
                    .build(),
            )
            .service(
                ServiceBuilder::new(self.config.callback_path.as_str())
                    .name("oauth_callback")
                    .filter(crate::router::filter::method::GET.clone())
                    .handler(Arc::new(callback))
                    .build(),
            );

        builder.wrap(Arc::new(self.session_manager.unwrap_or_default()))
    }

    pub fn build(self) -> crate::server::Server {
        self.finish_oauth().build()
    }
}

impl ServerBuilder {
    pub fn enable_oauth(self, provider: OAUTH) -> OAuthServerBuilder {
        OAuthServerBuilder {
            builder: self,
            config: OAuthRouteConfig::new(provider),
            session_manager: Some(SessionManager::default()),
        }
    }
}

#[derive(Clone)]
struct OAuthLoginService {
    config: OAuthRouteConfig,
}

impl Service for OAuthLoginService {
    fn name(&self) -> &str {
        "oauth_login"
    }

    fn serve<'a>(
        &'a self,
        request: &'a mut Request,
    ) -> Pin<Box<dyn Future<Output = Result<Response, PortfuError>> + 'a + Send>> {
        Box::pin(async move {
            let session = session_from_request(request)?;
            let client = OAuthClient::new(self.config.config.clone())?;
            client
                .authorization_url(&session, &self.config.scopes)
                .await
                .map(redirect)
        })
    }
}

#[derive(Clone)]
struct OAuthCallbackService {
    config: OAuthRouteConfig,
}

impl Service for OAuthCallbackService {
    fn name(&self) -> &str {
        "oauth_callback"
    }

    fn serve<'a>(
        &'a self,
        request: &'a mut Request,
    ) -> Pin<Box<dyn Future<Output = Result<Response, PortfuError>> + 'a + Send>> {
        Box::pin(async move {
            let session = session_from_request(request)?;
            let callback = <Query<OAuthCallbackQuery> as FromRequest<Request>>::try_from(request)
                .await?
                .into_inner();
            let client = OAuthClient::new(self.config.config.clone())?;
            match client.exchange_code(&session, &callback).await {
                Ok(token) => {
                    let decision = self.config.evaluate_policy(token).await?;
                    if !decision.allow {
                        let mut session = session.write().await;
                        let _ = session.data.remove::<SessionOAuthToken>();
                        let _ = session.data.remove::<SessionOAuthIdentity>();
                        drop(session);
                        if let Some(failure) = &self.config.failure_redirect {
                            return Ok(redirect(failure));
                        }
                        return Ok(Response::from_status_and_message(
                            StatusCode::FORBIDDEN,
                            decision
                                .message
                                .unwrap_or_else(|| "OAuth policy rejected request".to_string()),
                        ));
                    }
                    session
                        .write()
                        .await
                        .data
                        .insert(SessionOAuthIdentity(decision.identity.clone()));
                    if let Some(success) = &self.config.success_redirect {
                        Ok(redirect(success))
                    } else {
                        Ok(Response::json(decision.identity))
                    }
                }
                Err(e) => {
                    if let Some(failure) = &self.config.failure_redirect {
                        Ok(redirect(failure))
                    } else {
                        Err(e)
                    }
                }
            }
        })
    }
}

pub fn session_from_request(request: &Request) -> Result<Arc<RwLock<Session>>, PortfuError> {
    request
        .get::<Arc<RwLock<Session>>>()
        .cloned()
        .ok_or_else(|| {
            PortfuError::Internal("OAuth requires SessionManager middleware".to_string())
        })
}

impl FromRequest<Request> for OAuthToken {
    type Error = PortfuError;

    fn try_from<'a>(
        value: &'a mut Request,
    ) -> Pin<Box<dyn Future<Output = Result<Self, Self::Error>> + 'a + Send + Sync>> {
        Box::pin(async move {
            let session = session_from_request(value)?;
            session
                .read()
                .await
                .data
                .get::<SessionOAuthToken>()
                .map(|token| token.0.clone())
                .ok_or_else(|| {
                    PortfuError::Parsing("Failed to find OAuth token in active session".to_string())
                })
        })
    }
}

impl FromRequest<Request> for OAuthIdentity {
    type Error = PortfuError;

    fn try_from<'a>(
        value: &'a mut Request,
    ) -> Pin<Box<dyn Future<Output = Result<Self, Self::Error>> + 'a + Send + Sync>> {
        Box::pin(async move {
            let session = session_from_request(value)?;
            session
                .read()
                .await
                .data
                .get::<SessionOAuthIdentity>()
                .map(|identity| identity.0.clone())
                .ok_or_else(|| {
                    PortfuError::Parsing(
                        "Failed to find OAuth identity in active session".to_string(),
                    )
                })
        })
    }
}

fn default_scopes(provider: OAUTH) -> Vec<String> {
    match provider {
        OAUTH::KEYCLOAK => ["openid", "profile", "email"]
            .into_iter()
            .map(str::to_string)
            .collect(),
        OAUTH::GITHUB => ["read:user", "user:email", "read:org"]
            .into_iter()
            .map(str::to_string)
            .collect(),
        OAUTH::CUSTOM => Vec::new(),
    }
}

impl OAuthRouteConfig {
    pub async fn evaluate_policy(
        &self,
        token: OAuthToken,
    ) -> Result<OAuthPolicyDecision, PortfuError> {
        let userinfo = self.fetch_userinfo(&token).await?;
        self.evaluate_userinfo(token, userinfo).await
    }

    pub async fn evaluate_userinfo(
        &self,
        token: OAuthToken,
        userinfo: Value,
    ) -> Result<OAuthPolicyDecision, PortfuError> {
        let mut identity = identity_from_userinfo(
            self.provider,
            token.scopes.clone(),
            userinfo.clone(),
            self.policy.default_role.clone(),
        );
        let role_values = role_values(self.provider, &userinfo);
        let user_values = user_values(&identity);
        let org_values = organization_values(self.provider, &userinfo);

        if !self.policy.allowed_users.is_empty()
            && !has_match(&self.policy.allowed_users, &user_values)
        {
            return Ok(OAuthPolicyDecision::deny(
                identity,
                "OAuth user is not allowed",
            ));
        }
        if !self.policy.allowed_roles.is_empty()
            && !has_match(&self.policy.allowed_roles, &role_values)
        {
            return Ok(OAuthPolicyDecision::deny(
                identity,
                "OAuth role is not allowed",
            ));
        }
        if !self.policy.allowed_organizations.is_empty()
            && !has_match(&self.policy.allowed_organizations, &org_values)
        {
            return Ok(OAuthPolicyDecision::deny(
                identity,
                "OAuth organization is not allowed",
            ));
        }

        if has_match(&self.policy.admin_users, &user_values)
            || has_match(&self.policy.admin_roles, &role_values)
        {
            identity.role = Some("admin".to_string());
        }

        let decision = OAuthPolicyDecision::allow(identity);
        if let Some(handler) = &self.policy.handler {
            handler
                .evaluate(OAuthPolicyContext {
                    provider: self.provider,
                    token,
                    identity: decision.identity.clone(),
                    userinfo,
                })
                .await
        } else {
            Ok(decision)
        }
    }

    async fn fetch_userinfo(&self, token: &OAuthToken) -> Result<Value, PortfuError> {
        match self.provider {
            OAUTH::KEYCLOAK => {
                let Some(userinfo_url) = &self.policy.userinfo_url else {
                    return Ok(Value::Null);
                };
                get_json(userinfo_url, token).await
            }
            OAUTH::GITHUB => {
                let base = self
                    .policy
                    .api_base_url
                    .as_deref()
                    .unwrap_or("https://api.github.com")
                    .trim_end_matches('/');
                let user = get_json(format!("{base}/user"), token).await?;
                let emails = get_json(format!("{base}/user/emails"), token)
                    .await
                    .unwrap_or(Value::Array(vec![]));
                let orgs = get_json(format!("{base}/user/orgs"), token)
                    .await
                    .unwrap_or(Value::Array(vec![]));
                Ok(serde_json::json!({
                    "user": user,
                    "emails": emails,
                    "orgs": orgs,
                }))
            }
            OAUTH::CUSTOM => {
                if let Some(userinfo_url) = &self.policy.userinfo_url {
                    get_json(userinfo_url, token).await
                } else {
                    Ok(Value::Null)
                }
            }
        }
    }
}

async fn get_json(url: impl AsRef<str>, token: &OAuthToken) -> Result<Value, PortfuError> {
    reqwest::Client::new()
        .get(url.as_ref())
        .bearer_auth(token.access_token.as_str())
        .header(reqwest::header::USER_AGENT, "portfu/2.0")
        .send()
        .await
        .map_err(|e| PortfuError::Internal(format!("Failed OAuth userinfo request: {e}")))?
        .error_for_status()
        .map_err(|e| PortfuError::Internal(format!("OAuth userinfo request failed: {e}")))?
        .json::<Value>()
        .await
        .map_err(|e| PortfuError::Parsing(format!("Failed to parse OAuth userinfo JSON: {e}")))
}

fn identity_from_userinfo(
    provider: OAUTH,
    scopes: Vec<String>,
    userinfo: Value,
    default_role: Option<String>,
) -> OAuthIdentity {
    match provider {
        OAUTH::GITHUB => {
            let user = userinfo.get("user").unwrap_or(&userinfo);
            OAuthIdentity {
                provider,
                subject: string_value(user, "id").unwrap_or_else(|| "github".to_string()),
                username: string_value(user, "login"),
                email: string_value(user, "email").or_else(|| primary_github_email(&userinfo)),
                role: default_role,
                scopes,
                raw: userinfo,
            }
        }
        OAUTH::KEYCLOAK | OAUTH::CUSTOM => OAuthIdentity {
            provider,
            subject: string_value(&userinfo, "sub")
                .or_else(|| string_value(&userinfo, "id"))
                .unwrap_or_else(|| "oauth".to_string()),
            username: string_value(&userinfo, "preferred_username")
                .or_else(|| string_value(&userinfo, "username"))
                .or_else(|| string_value(&userinfo, "name")),
            email: string_value(&userinfo, "email"),
            role: default_role,
            scopes,
            raw: userinfo,
        },
    }
}

fn user_values(identity: &OAuthIdentity) -> Vec<String> {
    let mut values = vec![identity.subject.clone()];
    if let Some(username) = &identity.username {
        values.push(username.clone());
    }
    if let Some(email) = &identity.email {
        values.push(email.clone());
    }
    values
}

fn role_values(provider: OAUTH, userinfo: &Value) -> Vec<String> {
    match provider {
        OAUTH::KEYCLOAK => {
            let mut roles = Vec::new();
            if let Some(realm_roles) = userinfo
                .get("realm_access")
                .and_then(|v| v.get("roles"))
                .and_then(Value::as_array)
            {
                roles.extend(
                    realm_roles
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_string)),
                );
            }
            if let Some(resource_access) =
                userinfo.get("resource_access").and_then(Value::as_object)
            {
                for resource in resource_access.values() {
                    if let Some(resource_roles) = resource.get("roles").and_then(Value::as_array) {
                        roles.extend(
                            resource_roles
                                .iter()
                                .filter_map(|v| v.as_str().map(str::to_string)),
                        );
                    }
                }
            }
            roles
        }
        _ => Vec::new(),
    }
}

fn organization_values(provider: OAUTH, userinfo: &Value) -> Vec<String> {
    match provider {
        OAUTH::GITHUB => userinfo
            .get("orgs")
            .and_then(Value::as_array)
            .map(|orgs| {
                orgs.iter()
                    .flat_map(|org| [string_value(org, "id"), string_value(org, "login")])
                    .flatten()
                    .collect()
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn primary_github_email(userinfo: &Value) -> Option<String> {
    userinfo
        .get("emails")
        .and_then(Value::as_array)?
        .iter()
        .find(|email| {
            email
                .get("primary")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                && email
                    .get("verified")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
        })
        .and_then(|email| string_value(email, "email"))
}

fn string_value(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(|v| {
        v.as_str()
            .map(str::to_string)
            .or_else(|| v.as_u64().map(|v| v.to_string()))
            .or_else(|| v.as_i64().map(|v| v.to_string()))
    })
}

fn has_match(allowed: &[String], actual: &[String]) -> bool {
    allowed.is_empty()
        || allowed.iter().any(|allowed| {
            actual
                .iter()
                .any(|actual| actual.eq_ignore_ascii_case(allowed))
        })
}

pub struct OAuthClient {
    oauth_client: BasicClient,
}

impl OAuthClient {
    pub fn new(config: OAuthConfig) -> Result<Self, PortfuError> {
        let oauth_client = BasicClient::new(
            ClientId::new(config.client_id),
            Some(ClientSecret::new(config.client_secret)),
            AuthUrl::new(config.auth_url).map_err(|e| {
                PortfuError::Parsing(format!("Invalid oauth auth_url configuration: {e}"))
            })?,
            Some(TokenUrl::new(config.token_url).map_err(|e| {
                PortfuError::Parsing(format!("Invalid oauth token_url configuration: {e}"))
            })?),
        )
        .set_redirect_uri(RedirectUrl::new(config.redirect_url).map_err(|e| {
            PortfuError::Parsing(format!("Invalid oauth redirect_url configuration: {e}"))
        })?);
        Ok(Self { oauth_client })
    }

    pub async fn authorization_url(
        &self,
        session: &Arc<RwLock<Session>>,
        scopes: &[impl AsRef<str>],
    ) -> Result<String, PortfuError> {
        let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
        let mut auth = self
            .oauth_client
            .authorize_url(CsrfToken::new_random)
            .set_pkce_challenge(pkce_challenge);
        for scope in scopes {
            auth = auth.add_scope(Scope::new(scope.as_ref().to_string()));
        }
        let (url, csrf) = auth.url();
        let mut session = session.write().await;
        session
            .data
            .insert(SessionCsrfToken(csrf.secret().to_string()));
        session
            .data
            .insert(SessionPkceVerifier(pkce_verifier.secret().to_string()));
        Ok(url.to_string())
    }

    pub async fn exchange_code(
        &self,
        session: &Arc<RwLock<Session>>,
        callback: &OAuthCallbackQuery,
    ) -> Result<OAuthToken, PortfuError> {
        let (csrf, verifier) = {
            let session = session.read().await;
            let csrf = session
                .data
                .get::<SessionCsrfToken>()
                .map(|v| v.0.clone())
                .ok_or_else(|| {
                    PortfuError::Parsing(
                        "Missing oauth csrf token in session. Start oauth flow first.".to_string(),
                    )
                })?;
            let verifier = session
                .data
                .get::<SessionPkceVerifier>()
                .map(|v| v.0.clone())
                .ok_or_else(|| {
                    PortfuError::Parsing(
                        "Missing oauth pkce verifier in session. Start oauth flow first."
                            .to_string(),
                    )
                })?;
            (csrf, verifier)
        };

        if csrf != callback.state {
            return Err(PortfuError::Parsing(
                "OAuth state mismatch. Potential CSRF attempt.".to_string(),
            ));
        }

        let token = self
            .oauth_client
            .exchange_code(AuthorizationCode::new(callback.code.clone()))
            .set_pkce_verifier(PkceCodeVerifier::new(verifier))
            .request_async(async_http_client)
            .await
            .map_err(|e| PortfuError::Internal(format!("Failed oauth token exchange: {e}")))?;

        let mapped = OAuthToken {
            access_token: token.access_token().secret().to_string(),
            refresh_token: token.refresh_token().map(|v| v.secret().to_string()),
            token_type: token.token_type().as_ref().to_string(),
            expires_in_seconds: token.expires_in().map(|v| v.as_secs()),
            scopes: token
                .scopes()
                .map(|s| s.iter().map(|v| v.as_ref().to_string()).collect())
                .unwrap_or_default(),
        };

        let mut session = session.write().await;
        session.data.insert(SessionOAuthToken(mapped.clone()));
        let _ = session.data.remove::<SessionCsrfToken>();
        let _ = session.data.remove::<SessionPkceVerifier>();
        Ok(mapped)
    }

    pub async fn session_token(&self, session: &Arc<RwLock<Session>>) -> Option<OAuthToken> {
        session
            .read()
            .await
            .data
            .get::<SessionOAuthToken>()
            .map(|v| v.0.clone())
    }
}

pub fn redirect(location: impl AsRef<str>) -> Response {
    let mut response = Response::from_status_and_message(StatusCode::FOUND, "");
    if let Ok(value) = HeaderValue::from_str(location.as_ref()) {
        response.headers_mut().insert(LOCATION, value);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::{OAUTH, OAuthConfig, OAuthPolicyDecision, OAuthRouteConfig, OAuthToken, redirect};
    use serde_json::json;
    use std::sync::{Mutex, OnceLock};

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn env_lock() -> &'static Mutex<()> {
        ENV_LOCK.get_or_init(|| Mutex::new(()))
    }

    fn clear(prefix: &str) {
        // SAFETY: guarded by a process-wide mutex to avoid concurrent env mutation in tests.
        unsafe {
            std::env::remove_var(format!("{prefix}_CLIENT_ID"));
            std::env::remove_var(format!("{prefix}_CLIENT_SECRET"));
            std::env::remove_var(format!("{prefix}_AUTH_URL"));
            std::env::remove_var(format!("{prefix}_TOKEN_URL"));
            std::env::remove_var(format!("{prefix}_REDIRECT_URL"));
        }
    }

    #[test]
    fn oauth_config_reads_prefixed_env() {
        let _guard = env_lock().lock().expect("failed to lock env mutex");
        let prefix = "PORTFU_TEST_OAUTH";
        clear(prefix);
        // SAFETY: guarded by a process-wide mutex to avoid concurrent env mutation in tests.
        unsafe {
            std::env::set_var(format!("{prefix}_CLIENT_ID"), "client");
            std::env::set_var(format!("{prefix}_CLIENT_SECRET"), "secret");
            std::env::set_var(format!("{prefix}_AUTH_URL"), "https://example.com/auth");
            std::env::set_var(format!("{prefix}_TOKEN_URL"), "https://example.com/token");
            std::env::set_var(
                format!("{prefix}_REDIRECT_URL"),
                "https://example.com/callback",
            );
        }
        let config = OAuthConfig::from_env(prefix).expect("expected config from env");
        assert_eq!(config.client_id, "client");
        assert_eq!(config.client_secret, "secret");
        clear(prefix);
    }

    #[test]
    fn redirect_builds_found_with_location() {
        let response = redirect("https://example.com/next");
        assert_eq!(response.status(), http::StatusCode::FOUND);
        assert_eq!(
            response
                .headers()
                .get(http::header::LOCATION)
                .and_then(|v| v.to_str().ok()),
            Some("https://example.com/next")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn keycloak_policy_allows_roles_and_marks_admins() {
        let mut config = OAuthRouteConfig::new(OAUTH::KEYCLOAK);
        config.policy.allowed_roles.push("user".to_string());
        config.policy.admin_roles.push("admin".to_string());
        let decision = config
            .evaluate_userinfo(
                token(),
                json!({
                    "sub": "abc",
                    "preferred_username": "ada",
                    "email": "ada@example.com",
                    "realm_access": {
                        "roles": ["user", "admin"]
                    }
                }),
            )
            .await
            .expect("policy should evaluate");

        assert!(decision.allow);
        assert_eq!(decision.identity.subject, "abc");
        assert_eq!(decision.identity.username.as_deref(), Some("ada"));
        assert_eq!(decision.identity.role.as_deref(), Some("admin"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn github_policy_checks_users_and_organizations() {
        let mut config = OAuthRouteConfig::new(OAUTH::GITHUB);
        config.policy.allowed_users.push("42".to_string());
        config
            .policy
            .allowed_organizations
            .push("galactechs".to_string());
        let decision = config
            .evaluate_userinfo(
                token(),
                json!({
                    "user": {
                        "id": 42,
                        "login": "ada",
                        "email": null
                    },
                    "emails": [
                        {"email": "ada@example.com", "primary": true, "verified": true}
                    ],
                    "orgs": [
                        {"id": 7, "login": "galactechs"}
                    ]
                }),
            )
            .await
            .expect("policy should evaluate");

        assert!(decision.allow);
        assert_eq!(decision.identity.subject, "42");
        assert_eq!(decision.identity.email.as_deref(), Some("ada@example.com"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn custom_policy_callback_can_reject_provider_data() {
        let mut config = OAuthRouteConfig::new(OAUTH::CUSTOM);
        config.policy.handler = Some(std::sync::Arc::new(
            |context: super::OAuthPolicyContext| async move {
                Ok(OAuthPolicyDecision::deny(
                    context.identity,
                    "custom rejection",
                ))
            },
        ));
        let decision = config
            .evaluate_userinfo(token(), json!({"sub": "custom-user"}))
            .await
            .expect("policy should evaluate");

        assert!(!decision.allow);
        assert_eq!(decision.message.as_deref(), Some("custom rejection"));
    }

    fn token() -> OAuthToken {
        OAuthToken {
            access_token: "access".to_string(),
            refresh_token: None,
            token_type: "Bearer".to_string(),
            expires_in_seconds: None,
            scopes: vec!["openid".to_string()],
        }
    }
}
