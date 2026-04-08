use crate::error::PortfuError;
use crate::service::response::Response;
use crate::wrappers::sessions::Session;
use http::header::LOCATION;
use http::{HeaderValue, StatusCode};
use oauth2::basic::BasicClient;
use oauth2::reqwest::async_http_client;
use oauth2::{
    AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, Scope, TokenResponse, TokenUrl,
};
use serde::{Deserialize, Serialize};
use std::env;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone, Debug)]
struct SessionCsrfToken(String);

#[derive(Clone, Debug)]
struct SessionPkceVerifier(String);

#[derive(Clone, Debug)]
pub struct SessionOAuthToken(pub OAuthToken);

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

#[derive(Clone, Debug)]
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
    use super::{OAuthConfig, redirect};
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
}
