use crate::error::PortfuError;
use crate::service::request::{FromRequest, Request};
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientIdentity {
    pub leaf_der: Arc<[u8]>,
    /// Intermediates supplied by the peer, excluding the leaf certificate.
    pub chain_der: Arc<[Arc<[u8]>]>,
    pub sha256_fingerprint: [u8; 32],
    pub verified_by: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NegotiatedTlsVersion {
    Tls12,
    Tls13,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectionInfo {
    pub peer_addr: SocketAddr,
    pub local_addr: SocketAddr,
    pub tls_version: Option<NegotiatedTlsVersion>,
    pub client_identity: Option<ClientIdentity>,
}

impl ConnectionInfo {
    pub fn plaintext(peer_addr: SocketAddr, local_addr: SocketAddr) -> Self {
        Self {
            peer_addr,
            local_addr,
            tls_version: None,
            client_identity: None,
        }
    }
}

impl FromRequest<Request> for ConnectionInfo {
    type Error = PortfuError;

    fn try_from<'a>(
        value: &'a mut Request,
    ) -> Pin<Box<dyn Future<Output = Result<Self, Self::Error>> + 'a + Send + Sync>> {
        Box::pin(async move {
            value.get::<ConnectionInfo>().cloned().ok_or_else(|| {
                PortfuError::Internal("ConnectionInfo is missing from the request".to_string())
            })
        })
    }
}

impl FromRequest<Request> for ClientIdentity {
    type Error = PortfuError;

    fn try_from<'a>(
        value: &'a mut Request,
    ) -> Pin<Box<dyn Future<Output = Result<Self, Self::Error>> + 'a + Send + Sync>> {
        Box::pin(async move {
            value
                .get::<ClientIdentity>()
                .cloned()
                .ok_or_else(|| PortfuError::Unauthorized("Client certificate required".to_string()))
        })
    }
}
