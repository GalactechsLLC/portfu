use crate::auth::oauth::{OAuthIdentity, OAuthToken, SessionOAuthIdentity, SessionOAuthToken};
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

struct OAuthRolesFilter {
    name: String,
    roles: Vec<String>,
    mode: FilterMode,
}

impl FilterFn for OAuthRolesFilter {
    fn name(&self) -> &str {
        self.name.as_str()
    }

    fn filter<'a>(
        &'a self,
        request: &'a Request,
    ) -> Pin<Box<dyn Future<Output = FilterResult> + 'a + Send + Sync>> {
        Box::pin(async move {
            let Some(identity) = oauth_identity(request).await else {
                return FilterResult::Block;
            };
            match self.mode {
                FilterMode::Any => self
                    .roles
                    .iter()
                    .any(|role| has_identity_value(&identity.roles, role))
                    .into(),
                FilterMode::All => self
                    .roles
                    .iter()
                    .all(|role| has_identity_value(&identity.roles, role))
                    .into(),
            }
        })
    }
}

struct OAuthGroupsFilter {
    name: String,
    groups: Vec<String>,
    mode: FilterMode,
}

impl FilterFn for OAuthGroupsFilter {
    fn name(&self) -> &str {
        self.name.as_str()
    }

    fn filter<'a>(
        &'a self,
        request: &'a Request,
    ) -> Pin<Box<dyn Future<Output = FilterResult> + 'a + Send + Sync>> {
        Box::pin(async move {
            let Some(identity) = oauth_identity(request).await else {
                return FilterResult::Block;
            };
            match self.mode {
                FilterMode::Any => self
                    .groups
                    .iter()
                    .any(|group| has_identity_value(&identity.groups, group))
                    .into(),
                FilterMode::All => self
                    .groups
                    .iter()
                    .all(|group| has_identity_value(&identity.groups, group))
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

pub fn oauth_role<S: Into<String>>(role: S) -> Arc<Filter> {
    oauth_all_roles([role])
}

pub fn oauth_any_role<I, S>(roles: I) -> Arc<Filter>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    oauth_roles(roles, FilterMode::Any)
}

pub fn oauth_all_roles<I, S>(roles: I) -> Arc<Filter>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    oauth_roles(roles, FilterMode::All)
}

pub fn oauth_group<S: Into<String>>(group: S) -> Arc<Filter> {
    oauth_all_groups([group])
}

pub fn oauth_any_group<I, S>(groups: I) -> Arc<Filter>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    oauth_groups(groups, FilterMode::Any)
}

pub fn oauth_all_groups<I, S>(groups: I) -> Arc<Filter>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    oauth_groups(groups, FilterMode::All)
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

fn oauth_roles<I, S>(roles: I, mode: FilterMode) -> Arc<Filter>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let roles = roles.into_iter().map(Into::into).collect::<Vec<_>>();
    Arc::new(Filter {
        name: "oauth_roles".to_string(),
        mode: FilterMode::All,
        filter_functions: vec![Arc::new(OAuthRolesFilter {
            name: "oauth_roles".to_string(),
            roles,
            mode,
        })],
    })
}

fn oauth_groups<I, S>(groups: I, mode: FilterMode) -> Arc<Filter>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let groups = groups.into_iter().map(Into::into).collect::<Vec<_>>();
    Arc::new(Filter {
        name: "oauth_groups".to_string(),
        mode: FilterMode::All,
        filter_functions: vec![Arc::new(OAuthGroupsFilter {
            name: "oauth_groups".to_string(),
            groups,
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

async fn oauth_identity(request: &Request) -> Option<OAuthIdentity> {
    let session = active_session(request).await?;
    session
        .read()
        .await
        .data
        .get::<SessionOAuthIdentity>()
        .map(|identity| identity.0.clone())
}

async fn active_session(request: &Request) -> Option<Arc<RwLock<Session>>> {
    if let Some(session) = request.get::<Arc<RwLock<Session>>>() {
        Some(session.clone())
    } else {
        get_session_from_request(request).await
    }
}

fn has_identity_value(values: &[String], expected: &str) -> bool {
    values
        .iter()
        .any(|actual| actual.eq_ignore_ascii_case(expected))
}
