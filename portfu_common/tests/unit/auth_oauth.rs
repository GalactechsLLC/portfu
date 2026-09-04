use super::{OAUTH, OAuthConfig, OAuthPolicyDecision, OAuthRouteConfig, OAuthToken, redirect};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};
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
                },
                "resource_access": {
                    "portfu": {
                        "roles": ["editor"]
                    }
                },
                "groups": ["/engineering", "/platform"]
            }),
        )
        .await
        .expect("policy should evaluate");

    assert!(decision.allow);
    assert_eq!(decision.identity.subject, "abc");
    assert_eq!(decision.identity.username.as_deref(), Some("ada"));
    assert_eq!(
        decision.identity.roles,
        vec![
            "user".to_string(),
            "admin".to_string(),
            "editor".to_string(),
            "portfu:editor".to_string()
        ]
    );
    assert_eq!(
        decision.identity.groups,
        vec!["/engineering".to_string(), "/platform".to_string()]
    );
}

#[tokio::test(flavor = "current_thread")]
async fn keycloak_policy_maps_access_token_claims_when_userinfo_is_empty() {
    let mut config = OAuthRouteConfig::new(OAUTH::KEYCLOAK);
    config
        .policy
        .allowed_roles
        .push("portfu:writer".to_string());
    config
        .policy
        .allowed_organizations
        .push("/engineering".to_string());
    let decision = config
        .evaluate_userinfo(
            token_with_access_token(jwt_token(json!({
                "sub": "token-subject",
                "preferred_username": "token-user",
                "email": "token@example.com",
                "realm_access": {
                    "roles": ["user"]
                },
                "resource_access": {
                    "portfu": {
                        "roles": ["writer"]
                    }
                },
                "groups": ["/engineering"]
            }))),
            Value::Null,
        )
        .await
        .expect("policy should evaluate");

    assert!(decision.allow);
    assert_eq!(decision.identity.subject, "token-subject");
    assert_eq!(decision.identity.username.as_deref(), Some("token-user"));
    assert_eq!(
        decision.identity.email.as_deref(),
        Some("token@example.com")
    );
    assert_eq!(
        decision.identity.roles,
        vec![
            "user".to_string(),
            "writer".to_string(),
            "portfu:writer".to_string()
        ]
    );
    assert_eq!(decision.identity.groups, vec!["/engineering".to_string()]);
}

#[tokio::test(flavor = "current_thread")]
async fn oauth_identity_merges_scopes_from_access_token_claims() {
    let config = OAuthRouteConfig::new(OAUTH::KEYCLOAK);
    let decision = config
        .evaluate_userinfo(
            token_with_access_token_and_scopes(
                jwt_token(json!({
                    "sub": "token-subject",
                    "scope": "openid profile",
                    "scp": ["email"],
                    "scopes": ["profile", "offline_access"]
                })),
                vec!["openid".to_string()],
            ),
            Value::Null,
        )
        .await
        .expect("policy should evaluate");

    assert_eq!(
        decision.identity.scopes,
        vec![
            "openid".to_string(),
            "profile".to_string(),
            "email".to_string(),
            "offline_access".to_string()
        ]
    );
}

#[tokio::test(flavor = "current_thread")]
async fn custom_policy_maps_roles_and_groups_from_scalar_or_array_values() {
    let config = OAuthRouteConfig::new(OAUTH::CUSTOM);
    let decision = config
        .evaluate_userinfo(
            token(),
            json!({
                "sub": "abc",
                "roles": ["editor", "reviewer"],
                "role": "editor",
                "group": "/ops",
                "groups": ["/engineering", "/ops"]
            }),
        )
        .await
        .expect("policy should evaluate");

    assert!(decision.allow);
    assert_eq!(
        decision.identity.roles,
        vec!["editor".to_string(), "reviewer".to_string()]
    );
    assert_eq!(
        decision.identity.groups,
        vec!["/engineering".to_string(), "/ops".to_string()]
    );
}

#[tokio::test(flavor = "current_thread")]
async fn default_roles_are_included_in_normalized_roles() {
    let mut config = OAuthRouteConfig::new(OAUTH::CUSTOM);
    config.policy.default_roles.push("member".to_string());
    let decision = config
        .evaluate_userinfo(
            token(),
            json!({
                "sub": "abc",
                "roles": ["editor"]
            }),
        )
        .await
        .expect("policy should evaluate");

    assert_eq!(
        decision.identity.roles,
        vec!["editor".to_string(), "member".to_string()]
    );
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
    assert_eq!(decision.identity.username.as_deref(), Some("ada"));
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

fn token_with_access_token(access_token: String) -> OAuthToken {
    OAuthToken {
        access_token,
        ..token()
    }
}

fn token_with_access_token_and_scopes(access_token: String, scopes: Vec<String>) -> OAuthToken {
    OAuthToken {
        access_token,
        scopes,
        ..token()
    }
}

fn jwt_token(claims: Value) -> String {
    let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"none"}"#);
    let payload = URL_SAFE_NO_PAD.encode(claims.to_string());
    format!("{header}.{payload}.")
}
