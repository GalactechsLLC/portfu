use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlsIdentity {
    pub domain: String,
    pub cert_chain_pem: Arc<[u8]>,
    pub private_key_pem: Arc<[u8]>,
}

impl TlsIdentity {
    pub fn new(
        domain: impl Into<String>,
        cert_chain_pem: impl AsRef<[u8]>,
        private_key_pem: impl AsRef<[u8]>,
    ) -> Self {
        Self {
            domain: domain.into(),
            cert_chain_pem: Arc::from(cert_chain_pem.as_ref()),
            private_key_pem: Arc::from(private_key_pem.as_ref()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustStore {
    pub name: String,
    pub certificates_pem: Arc<[u8]>,
}

impl TrustStore {
    pub fn new(name: impl Into<String>, certificates_pem: impl AsRef<[u8]>) -> Self {
        Self {
            name: name.into(),
            certificates_pem: Arc::from(certificates_pem.as_ref()),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ClientCertificateMode {
    #[default]
    Disabled,
    Optional,
    Required,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientAuthConfig {
    pub presentation: ClientCertificateMode,
    pub trust_stores: Vec<TrustStore>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TlsVersionPolicy {
    #[default]
    Tls12And13,
    Tls13Only,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TlsConfig {
    /// The first identity is the default certificate. All identities also participate in SNI.
    pub identities: Vec<TlsIdentity>,
    pub client_auth: ClientAuthConfig,
    pub versions: TlsVersionPolicy,
}

impl TlsConfig {
    pub fn new(identity: TlsIdentity) -> Self {
        Self {
            identities: vec![identity],
            ..Self::default()
        }
    }

    pub fn with_identity(mut self, identity: TlsIdentity) -> Self {
        self.identities.push(identity);
        self
    }

    pub fn client_auth(mut self, client_auth: ClientAuthConfig) -> Self {
        self.client_auth = client_auth;
        self
    }

    pub fn versions(mut self, versions: TlsVersionPolicy) -> Self {
        self.versions = versions;
        self
    }
}

#[derive(Debug)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub tls: Option<TlsConfig>,
    pub keep_alive: bool,
    pub half_close: bool,
    pub preserve_header_case: bool,
    pub max_buf_size: usize,
    pub backlog: u32,
    pub acceptors: usize,
    pub reuse_port: bool,
    pub websocket_shutdown_grace_period: Duration,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "localhost".to_string(),
            port: 8080,
            tls: None,
            keep_alive: true,
            half_close: true,
            preserve_header_case: true,
            max_buf_size: 1024 * 1024 * 2, // 2 MiB
            backlog: 1024,
            acceptors: std::thread::available_parallelism()
                .map(Into::into)
                .unwrap_or(1),
            reuse_port: true,
            websocket_shutdown_grace_period: Duration::from_secs(10),
        }
    }
}
