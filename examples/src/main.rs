use log::LevelFilter;
pub use portfu_updated::prelude::*;
use simple_logger::SimpleLogger;
use std::collections::HashMap;
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
        .global_state(RwLock::new(HashMap::<u64, String>::new()))
        .scoped_state("site-a", "Site A".to_string())
        .scoped_state("site-b", "Site B".to_string())
        .build()
        .run()
        .await
}

type MockDb = RwLock<HashMap<u64, String>>;

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
pub async fn example_ws_client(ws: WebSocketClient) -> Result<(), PortfuError> {
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
