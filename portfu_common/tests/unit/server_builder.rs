use crate::server::builder::ServerBuilder;
use crate::server::config::{TlsConfig, TlsIdentity};
use crate::service::builder::ServiceBuilder;
use crate::service::group::ServiceGroup;
use std::sync::{Arc, atomic::Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn env_lock() -> &'static Mutex<()> {
    ENV_LOCK.get_or_init(|| Mutex::new(()))
}

fn clear_env() {
    // SAFETY: guarded by a process-wide mutex to avoid concurrent env mutation in tests.
    unsafe {
        std::env::remove_var("PORTFU_HOST");
        std::env::remove_var("PORTFU_PORT");
        std::env::remove_var("PORTFU_BACKLOG");
        std::env::remove_var("PORTFU_ACCEPTORS");
        std::env::remove_var("PORTFU_REUSEPORT");
        std::env::remove_var("PORTFU_SSL_ENABLED");
    }
}

#[test]
fn from_env_keeps_defaults_when_env_is_missing() {
    let _guard = env_lock().lock().expect("failed to lock env mutex");
    clear_env();
    let builder = ServerBuilder::from_env();
    assert_eq!(builder.config.host, "localhost");
    assert_eq!(builder.config.port, 8080);
    assert_eq!(builder.config.backlog, 1024);
    assert!(builder.config.acceptors >= 1);
    assert!(builder.config.reuse_port);
    assert!(builder.config.tls.is_none());
}

#[test]
fn from_env_only_updates_values_that_exist() {
    let _guard = env_lock().lock().expect("failed to lock env mutex");
    clear_env();
    // SAFETY: guarded by a process-wide mutex to avoid concurrent env mutation in tests.
    unsafe {
        std::env::set_var("PORTFU_HOST", "0.0.0.0");
        std::env::set_var("PORTFU_PORT", "9090");
    }
    let builder = ServerBuilder::from_env();
    assert_eq!(builder.config.host, "0.0.0.0");
    assert_eq!(builder.config.port, 9090);
    assert_eq!(builder.config.backlog, 1024);
    assert!(builder.config.acceptors >= 1);
    assert!(builder.config.reuse_port);
    assert!(builder.config.tls.is_none());
    clear_env();
}

#[test]
fn from_env_parses_remaining_tunables_and_ignores_invalid_numbers() {
    let _guard = env_lock().lock().expect("failed to lock env mutex");
    clear_env();
    // SAFETY: guarded by a process-wide mutex to avoid concurrent env mutation in tests.
    unsafe {
        std::env::set_var("PORTFU_PORT", "not-a-port");
        std::env::set_var("PORTFU_BACKLOG", "2048");
        std::env::set_var("PORTFU_ACCEPTORS", "3");
        std::env::set_var("PORTFU_REUSEPORT", "false");
        std::env::set_var("PORTFU_SSL_ENABLED", "true");
    }
    let builder = ServerBuilder::from_env();
    assert_eq!(builder.config.port, 8080);
    assert_eq!(builder.config.backlog, 2048);
    assert_eq!(builder.config.acceptors, 3);
    assert!(!builder.config.reuse_port);
    assert!(builder.config.tls.is_some());
    clear_env();
}

#[test]
fn service_group_preserves_registration_order() {
    let server = ServerBuilder::new()
        .service(ServiceBuilder::new("/first").name("first").build())
        .service_group(
            ServiceGroup::new()
                .service(ServiceBuilder::new("/second").name("second").build())
                .service(ServiceBuilder::new("/third").name("third").build()),
        )
        .service(ServiceBuilder::new("/fourth").name("fourth").build())
        .build();

    assert_eq!(
        server
            .services
            .iter()
            .map(|service| service.name())
            .collect::<Vec<_>>(),
        vec!["first", "second", "third", "fourth"]
    );
}

#[tokio::test]
async fn service_group_merges_state_into_the_default_scope() {
    let server = ServerBuilder::new()
        .global_state("builder".to_string())
        .service_group(ServiceGroup::new().shared_state("first group".to_string()))
        .service_group(ServiceGroup::new().shared_state("last group".to_string()))
        .build();

    let state = server.scoped_state.read().await;
    assert_eq!(
        state
            .get("default")
            .and_then(|extensions| extensions.get::<Arc<String>>())
            .map(|value| value.as_str()),
        Some("last group")
    );
}

#[tokio::test]
async fn fluent_builder_methods_store_config_and_scoped_state() {
    let default_identity = TlsIdentity::new("example.test", "cert.pem", "key.pem");
    let sni_identity = TlsIdentity::new("api.example.test", "cert2.pem", "key2.pem");
    let server = ServerBuilder::new()
        .host("0.0.0.0")
        .port(9090)
        .tls(TlsConfig::new(default_identity.clone()))
        .tls_identity(sni_identity.clone())
        .tls_handshake_timeout(Duration::from_secs(7))
        .http_header_read_timeout(Duration::from_secs(8))
        .trust_proxy_headers(true)
        .shutdown_grace_period(Duration::from_secs(30))
        .global_state("global".to_string())
        .scoped_state("tenant", 99_u32)
        .build();

    assert_eq!(server.config.host, "0.0.0.0");
    assert_eq!(server.config.port, 9090);
    assert_eq!(
        server.config.tls.as_ref().map(|tls| &tls.identities),
        Some(&vec![default_identity, sni_identity])
    );
    assert_eq!(server.config.shutdown_grace_period, Duration::from_secs(30));
    assert_eq!(
        server.config.tls.as_ref().unwrap().handshake_timeout,
        Duration::from_secs(7)
    );
    assert_eq!(
        server.config.http_header_read_timeout,
        Duration::from_secs(8)
    );
    assert!(server.config.trust_proxy_headers);
    assert!(server.run.load(Ordering::Relaxed));

    let state = server.scoped_state.read().await;
    assert_eq!(
        state
            .get("default")
            .and_then(|ext| ext.get::<Arc<String>>())
            .map(|value| value.as_str()),
        Some("global")
    );
    assert_eq!(
        state
            .get("tenant")
            .and_then(|ext| ext.get::<Arc<u32>>())
            .map(|value| **value),
        Some(99)
    );
}
