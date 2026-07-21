use crate::auth::oauth::{OAuthToken, SessionOAuthToken};
use crate::router::filter::traits::Filter as FilterFn;
use crate::router::filter::{Filter, FilterMode, FilterResult};
use crate::service::request::Request;
use crate::wrappers::sessions::{Session, get_session_from_request};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::RwLock;

struct SessionFilter;

impl FilterFn for SessionFilter {
    fn name(&self) -> &str {
        "session"
    }

    fn filter<'a>(
        &'a self,
        request: &'a Request,
    ) -> Pin<Box<dyn Future<Output = FilterResult> + 'a + Send + Sync>> {
        Box::pin(async move { active_session(request).await.is_some().into() })
    }
}

struct OAuthFilter;

impl FilterFn for OAuthFilter {
    fn name(&self) -> &str {
        "oauth"
    }

    fn filter<'a>(
        &'a self,
        request: &'a Request,
    ) -> Pin<Box<dyn Future<Output = FilterResult> + 'a + Send + Sync>> {
        Box::pin(async move { oauth_token(request).await.is_some().into() })
    }
}

struct OAuthScopesFilter {
    name: String,
    scopes: Vec<String>,
    mode: FilterMode,
}

impl FilterFn for OAuthScopesFilter {
    fn name(&self) -> &str {
        self.name.as_str()
    }

    fn filter<'a>(
        &'a self,
        request: &'a Request,
    ) -> Pin<Box<dyn Future<Output = FilterResult> + 'a + Send + Sync>> {
        Box::pin(async move {
            let Some(token) = oauth_token(request).await else {
                return FilterResult::Block;
            };
            match self.mode {
                FilterMode::Any => self
                    .scopes
                    .iter()
                    .any(|scope| token.scopes.iter().any(|s| s == scope))
                    .into(),
                FilterMode::All => self
                    .scopes
                    .iter()
                    .all(|scope| token.scopes.iter().any(|s| s == scope))
                    .into(),
            }
        })
    }
}

pub fn session() -> Arc<Filter> {
    Arc::new(Filter {
        name: "session".to_string(),
        mode: FilterMode::All,
        filter_functions: vec![Arc::new(SessionFilter)],
    })
}

pub fn oauth() -> Arc<Filter> {
    Arc::new(Filter {
        name: "oauth".to_string(),
        mode: FilterMode::All,
        filter_functions: vec![Arc::new(OAuthFilter)],
    })
}

pub fn oauth_scope<S: Into<String>>(scope: S) -> Arc<Filter> {
    oauth_all_scopes([scope])
}

pub fn oauth_any_scope<I, S>(scopes: I) -> Arc<Filter>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    oauth_scopes(scopes, FilterMode::Any)
}

pub fn oauth_all_scopes<I, S>(scopes: I) -> Arc<Filter>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    oauth_scopes(scopes, FilterMode::All)
}

fn oauth_scopes<I, S>(scopes: I, mode: FilterMode) -> Arc<Filter>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let scopes = scopes.into_iter().map(Into::into).collect::<Vec<_>>();
    Arc::new(Filter {
        name: "oauth_scopes".to_string(),
        mode: FilterMode::All,
        filter_functions: vec![Arc::new(OAuthScopesFilter {
            name: "oauth_scopes".to_string(),
            scopes,
            mode,
        })],
    })
}

async fn oauth_token(request: &Request) -> Option<OAuthToken> {
    let session = active_session(request).await?;
    session
        .read()
        .await
        .data
        .get::<SessionOAuthToken>()
        .map(|token| token.0.clone())
}

async fn active_session(request: &Request) -> Option<Arc<RwLock<Session>>> {
    if let Some(session) = request.get::<Arc<RwLock<Session>>>() {
        Some(session.clone())
    } else {
        get_session_from_request(request).await
    }
}
