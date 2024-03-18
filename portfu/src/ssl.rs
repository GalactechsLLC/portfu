use std::env;
use std::io::{BufReader, Error, ErrorKind};
use std::sync::Arc;
use crate::server::ServerConfig;
use rustls::{RootCertStore};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls_pemfile::{certs, read_one, Item};
use log::error;
use rustls::crypto::ring::sign::RsaSigningKey;
use rustls::sign::CertifiedKey;
use tokio_rustls::rustls::server::ResolvesServerCertUsingSni;

pub fn load_ssl_certs(config: &ServerConfig) -> Result<Arc<rustls::ServerConfig>, Error> {
    let (certs, key, root_certs) = if let Some(ssl_info) = &config.ssl_config {
        (
            load_certs(ssl_info.certs.as_bytes())?,
            load_private_key(ssl_info.key.as_bytes())?,
            load_certs(ssl_info.root_certs.as_bytes())?,
        )
    } else if let (Some(certs), Some(key), Some(root_certs)) = (
        env::var("SSL_CERTS").ok(),
        env::var("SSL_PRIVATE_KEY").ok(),
        env::var("SSL_ROOT_CERTS").ok(),
    ) {
        (
            load_certs(certs.as_bytes())?,
            load_private_key(key.as_bytes())?,
            load_certs(root_certs.as_bytes())?,
        )
    } else {
        return Err(Error::new(ErrorKind::InvalidInput, "Invalid SSL Config"))
    };
    let mut root_cert_store = RootCertStore::empty();
    for cert in root_certs {
        root_cert_store.add(cert).map_err(|e| {
            Error::new(
                ErrorKind::InvalidInput,
                format!("Invalid Root Cert for Server: {:?}", e),
            )
        })?;
    }
    let mut resolver = ResolvesServerCertUsingSni::new();
    let name = config.ssl_config.as_ref().map(|c| c.domain.as_str()).unwrap_or("localhost");

    let cer_key = CertifiedKey::new(certs, Arc::new(RsaSigningKey::new(&key).map_err(|e| {
        Error::new(
            ErrorKind::InvalidInput,
            format!("Private Key is not Valid SigningKey: {:?}", e),
        )
    })?));
    resolver.add(name, cer_key).map_err(|e| {
        Error::new(
            ErrorKind::InvalidInput,
            format!("Failed to add SSL Certs to Resolver: {:?}", e),
        )
    })?;
    let resolver = Arc::new(resolver);
    Ok(Arc::new(
        tokio_rustls::rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_cert_resolver(resolver)
    ))
}
pub fn load_certs(bytes: &[u8]) -> Result<Vec<CertificateDer<'static>>, Error> {
    let mut reader = BufReader::new(bytes);
    let certs = certs(&mut reader);
    Ok(certs.into_iter().flatten().collect())
}

pub fn load_private_key(bytes: &[u8]) -> Result<PrivateKeyDer<'static>, Error> {
    let mut reader = BufReader::new(bytes);
    for item in std::iter::from_fn(|| read_one(&mut reader).transpose()) {
        if let Some(item) = handle_item(item) {
            return Ok(item)
        }
    }
    Err(Error::new(ErrorKind::NotFound, "Private Key Not Found"))
}

fn handle_item(item: Result<Item, Error>) -> Option<PrivateKeyDer<'static>> {
    match item {
        Ok(Item::Pkcs8Key(key))  => {
            return Some(PrivateKeyDer::Pkcs8(key));
        }
        Ok(Item::Pkcs1Key(key)) => {
            return Some(PrivateKeyDer::Pkcs1(key));
        }
        Ok(Item::Sec1Key(key)) => {
            return Some(PrivateKeyDer::Sec1(key));
        }
        Ok(Item::X509Certificate(_)) => error!("Found Certificate, not Private Key"),
        Ok(Item::Crl(_)) => error!("Found Crl, not Private Key"),
        Ok(Item::Csr(_)) => error!("Found Csr, not Private Key"),
        _ => {
            error!("Unknown Item while loading private key")
        }
    }
    None
}