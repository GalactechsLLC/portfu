use log::LevelFilter;
pub use portfu::prelude::*;
use simple_logger::SimpleLogger;
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::RwLock;

#[static_files("assets", name = "embedded-assets")]
pub struct EmbeddedAssets;

#[static_files(
    "assets",
    name = "site-a-assets",
    scope = "site-a",
    domain = "site-a.local"
)]
pub struct SiteAAssets;

#[static_files(
    "assets",
    name = "site-b-assets",
    scope = "site-b",
    domain = "site-b.local"
)]
pub struct SiteBAssets;

#[files(
    format!("{}/assets", env!("CARGO_MANIFEST_DIR")),
    mount = "/dynamic",
    name = "dynamic-assets"
)]
pub struct DynamicAssets;

#[tokio::main(flavor = "multi_thread")]
pub async fn main() -> Result<(), PortfuError> {
    SimpleLogger::new()
        .with_level(LevelFilter::Info)
        .env()
        .init()
        .unwrap();

    ServerBuilder::from_env()
        .enable_metrics()
        .path("/metrics")
        .finish_metrics()
        .enable_rate_limits()
        .requests_per_window(120, 60)
        .request_size_limit(1024 * 1024)
        .body_read_timeout(Duration::from_secs(10))
        .finish_rate_limits()
        .enable_cors()
        .allow_all()
        .finish_cors()
        .enable_oauth(OAUTH::KEYCLOAK)
        .client_id(env_or("KEYCLOAK_CLIENT_ID", "portfu-example"))
        .client_secret(env_or("KEYCLOAK_CLIENT_SECRET", "example-secret"))
        .auth_url(env_or(
            "KEYCLOAK_AUTH_URL",
            "http://localhost:8081/realms/portfu/protocol/openid-connect/auth",
        ))
        .token_url(env_or(
            "KEYCLOAK_TOKEN_URL",
            "http://localhost:8081/realms/portfu/protocol/openid-connect/token",
        ))
        .userinfo_url(env_or(
            "KEYCLOAK_USERINFO_URL",
            "http://localhost:8081/realms/portfu/protocol/openid-connect/userinfo",
        ))
        .redirect_url(env_or(
            "PORTFU_OAUTH_REDIRECT_URL",
            "http://localhost:8080/oauth/callback",
        ))
        .scopes(["openid", "profile", "email"])
        .allowed_role("user")
        .admin_role("admin")
        .default_role("user")
        .success_redirect("/auth/success")
        .failure_redirect("/auth/failure")
        .finish_oauth()
        .global_state(RwLock::new(HashMap::<u64, String>::new()))
        .scoped_state("site-a", "Site A".to_string())
        .scoped_state("site-b", "Site B".to_string())
        .build()
        .run()
        .await
}

type MockDb = RwLock<HashMap<u64, String>>;

fn env_or(key: &str, fallback: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| fallback.to_string())
}

#[get("/", scope = "site-a", domain = "site-a.local")]
pub async fn site_a_home(site: State<String>) -> Result<String, PortfuError> {
    Ok(format!("{} home page", site.inner()))
}

#[get("/", scope = "site-b", domain = "site-b.local")]
pub async fn site_b_home(site: State<String>) -> Result<String, PortfuError> {
    Ok(format!("{} home page", site.inner()))
}

fn parse_user_id(path: &Path) -> Result<u64, PortfuError> {
    path.value()
        .parse::<u64>()
        .map_err(|e| PortfuError::Parsing(format!("Invalid user id: {e}")))
}

#[get("/users/{id}")]
pub async fn user_get(id: Path, db: State<MockDb>) -> Result<String, PortfuError> {
    let user_id = parse_user_id(&id)?;
    let users = db.read().await;
    match users.get(&user_id) {
        Some(name) => Ok(format!("user_id={user_id} name={name}")),
        None => Ok(format!("user_id={user_id} not found")),
    }
}

#[post("/users/{id}/{name}")]
pub async fn user_post(id: Path, name: Path, db: State<MockDb>) -> Result<String, PortfuError> {
    let user_id = parse_user_id(&id)?;
    let name = name.inner();
    let mut users = db.write().await;
    if let std::collections::hash_map::Entry::Vacant(e) = users.entry(user_id) {
        e.insert(name.clone());
        Ok(format!("created user_id={user_id} name={name}"))
    } else {
        Ok(format!("user_id={user_id} already exists"))
    }
}

#[put("/users/{id}/{name}")]
pub async fn user_put(id: Path, name: Path, db: State<MockDb>) -> Result<String, PortfuError> {
    let user_id = parse_user_id(&id)?;
    let name = name.inner();
    let mut users = db.write().await;
    users.insert(user_id, name.clone());
    Ok(format!("upserted user_id={user_id} name={name}"))
}

#[delete("/users/{id}")]
pub async fn user_delete(id: Path, db: State<MockDb>) -> Result<String, PortfuError> {
    let user_id = parse_user_id(&id)?;
    let mut users = db.write().await;
    match users.remove(&user_id) {
        Some(name) => Ok(format!("deleted user_id={user_id} name={name}")),
        None => Ok(format!("user_id={user_id} not found")),
    }
}

#[get("/public/cors")]
pub async fn cors_example() -> Result<String, PortfuError> {
    Ok("cors headers are added by the server wrapper".to_string())
}

#[get("/auth/session", filter = filters::auth::session())]
pub async fn session_guard(session: SessionState) -> Result<String, PortfuError> {
    let id = session.inner().read().await.id;
    Ok(format!("session_id={id}"))
}

#[get("/auth/success", filter = filters::auth::oauth())]
pub async fn oauth_success(identity: OAuthIdentity) -> Result<String, PortfuError> {
    Ok(format!(
        "oauth_user={} roles={}",
        identity.subject,
        identity.roles.join(",")
    ))
}

#[get(
    "/auth/profile",
    filter = filters::auth::oauth_any_scope(["profile", "read:user"])
)]
pub async fn oauth_profile(
    identity: OAuthIdentity,
    token: OAuthToken,
) -> Result<String, PortfuError> {
    Ok(format!(
        "oauth_user={} scopes={}",
        identity.subject,
        token.scopes.join(",")
    ))
}

#[get("/auth/admin", filter = filters::auth::oauth_scope("admin"))]
pub async fn oauth_admin(identity: OAuthIdentity) -> Result<String, PortfuError> {
    Ok(format!("admin oauth_user={}", identity.subject))
}

#[get("/auth/failure")]
pub async fn oauth_failure() -> Result<String, PortfuError> {
    Ok("oauth policy rejected the login".to_string())
}

#[head("/users/{id}")]
pub async fn user_head(id: Path, db: State<MockDb>) -> Result<http::Response<()>, PortfuError> {
    let user_id = parse_user_id(&id)?;
    let users = db.read().await;
    let status = if users.contains_key(&user_id) {
        http::StatusCode::OK
    } else {
        http::StatusCode::NOT_FOUND
    };
    http::Response::builder()
        .status(status)
        .body(())
        .map_err(|e| PortfuError::Internal(format!("Failed to build head response: {e}")))
}

#[task]
pub async fn seed_mock_db(db: State<MockDb>) -> Result<(), PortfuError> {
    let mut users = db.write().await;
    users.entry(1).or_insert_with(|| "Ada".to_string());
    users.entry(2).or_insert_with(|| "Linus".to_string());
    log::info!("seeded mock db with {} users", users.len());
    Ok(())
}

#[interval(1000)]
pub async fn db_metrics(db: State<MockDb>) -> Result<(), PortfuError> {
    let users = db.read().await;
    log::info!("db user count={}", users.len());
    Ok(())
}

#[websocket("/ws/echo")]
pub async fn echo_websocket(websocket: WebSocket) -> Result<(), PortfuError> {
    while let Some(message) = websocket
        .next_message()
        .await
        .map_err(|e| PortfuError::Internal(format!("Failed to read websocket frame: {e}")))?
    {
        if message.is_close() {
            break;
        }
        websocket
            .send(message)
            .await
            .map_err(|e| PortfuError::Internal(format!("Failed to write websocket frame: {e}")))?;
    }
    Ok(())
}

#[client_websocket("ws://127.0.0.1:8080/ws/echo")]
pub async fn example_ws_client(ws: ClientWebSocket) -> Result<(), PortfuError> {
    ws.send(Message::text("hello from macro client"))
        .await
        .map_err(|e| {
            PortfuError::Internal(format!("Failed to send client websocket frame: {e}"))
        })?;
    if let Some(frame) = ws
        .next_message()
        .await
        .map_err(|e| PortfuError::Internal(format!("Failed to read client websocket frame: {e}")))?
    {
        log::info!("client websocket received: {frame:?}");
    }
    Ok(())
}
