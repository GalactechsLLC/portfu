use serde::{Deserialize, Serialize};

#[derive(Default, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SslConfig {
    pub domain: String,
    pub key: String,
    pub certs: String,
    pub root_certs: String,
}

#[derive(Debug)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub ssl_config: Option<SslConfig>,
    pub client_ssl_config: Option<SslConfig>,
    pub keep_alive: bool,
    pub half_close: bool,
    pub preserve_header_case: bool,
    pub max_buf_size: usize,
}
impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "localhost".to_string(),
            port: 8080,
            ssl_config: None,
            client_ssl_config: None,
            keep_alive: true,
            half_close: true,
            preserve_header_case: true,
            max_buf_size: 1024 * 1024 * 2, //2 Mib
        }
    }
}
