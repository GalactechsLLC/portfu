#[derive(Default, Debug, Clone, PartialEq, Eq)]
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
    pub enable_ssl: bool,
    pub ssl_config: Option<SslConfig>,
    pub sni_ssl_configs: Vec<SslConfig>,
    pub client_ssl_config: Option<SslConfig>,
    pub keep_alive: bool,
    pub half_close: bool,
    pub preserve_header_case: bool,
    pub max_buf_size: usize,
    pub backlog: u32,
    pub acceptors: usize,
    pub reuse_port: bool,
}
impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "localhost".to_string(),
            port: 8080,
            enable_ssl: false,
            ssl_config: None,
            sni_ssl_configs: vec![],
            client_ssl_config: None,
            keep_alive: true,
            half_close: true,
            preserve_header_case: true,
            max_buf_size: 1024 * 1024 * 2, //2 Mib
            backlog: 1024,
            acceptors: std::thread::available_parallelism()
                .map(Into::into)
                .unwrap_or(1),
            reuse_port: true,
        }
    }
}
