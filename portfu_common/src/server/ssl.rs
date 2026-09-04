use crate::error::PortfuError;
use crate::server::config::{
    ClientCertificateMode, ServerConfig, TlsConfig, TlsIdentity, TlsVersionPolicy,
};
use crate::server::connection::{ClientIdentity, NegotiatedTlsVersion};
use rcgen::generate_simple_self_signed;
use rustls::client::danger::HandshakeSignatureValid;
use rustls::crypto::aws_lc_rs::default_provider;
use rustls::crypto::aws_lc_rs::sign::any_supported_type;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, DnsName, PrivateKeyDer, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::server::{ClientHello, ParsedCertificate, ResolvesServerCert, WebPkiClientVerifier};
use rustls::sign::CertifiedKey;
use rustls::{DigitallySignedStruct, DistinguishedName, RootCertStore, SignatureScheme};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::env;
use std::io::{Error, ErrorKind};
use std::sync::Arc;

pub struct LoadedTlsConfig {
    pub server_config: Arc<rustls::ServerConfig>,
    pub client_verifier: Option<Arc<NamedClientVerifier>>,
}

pub fn load_ssl_certs(config: &ServerConfig) -> Result<LoadedTlsConfig, PortfuError> {
    let provider = Arc::new(default_provider());
    let mut resolver = ResolvesServerCertUsingSniWithDefault::new();
    let tls = config.tls.clone().unwrap_or_default();

    let mut cert_configs = collect_server_certs(&tls);
    if cert_configs.is_empty() {
        cert_configs.push(default_localhost_cert()?);
    }

    for cert_config in cert_configs {
        let certs = load_certs(&cert_config.cert_chain_pem)?;
        let key = load_private_key(&cert_config.private_key_pem)?;
        let signing_key = any_supported_type(&key)
            .map_err(|e| PortfuError::Internal(format!("Private key is invalid: {e:?}")))?;
        let cert_key = CertifiedKey::new(certs, signing_key);
        resolver.add(cert_config.domain.as_str(), cert_key)?;
    }

    let versions: &[&'static rustls::SupportedProtocolVersion] = match tls.versions {
        TlsVersionPolicy::Tls12And13 => &[&rustls::version::TLS13, &rustls::version::TLS12],
        TlsVersionPolicy::Tls13Only => &[&rustls::version::TLS13],
    };
    let builder = rustls::ServerConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(versions)
        .map_err(|e| PortfuError::Internal(format!("Invalid TLS version policy: {e}")))?;
    let client_verifier = NamedClientVerifier::build(&tls, provider)?;
    let server_config = match &client_verifier {
        Some(verifier) => builder
            .with_client_cert_verifier(verifier.clone())
            .with_cert_resolver(Arc::new(resolver)),
        None => builder
            .with_no_client_auth()
            .with_cert_resolver(Arc::new(resolver)),
    };
    Ok(LoadedTlsConfig {
        server_config: Arc::new(server_config),
        client_verifier,
    })
}

fn collect_server_certs(config: &TlsConfig) -> Vec<TlsIdentity> {
    let mut certs = config.identities.clone();
    if certs.is_empty()
        && let (Some(certs_pem), Some(key_pem)) =
            (env::var("SSL_CERTS").ok(), env::var("SSL_PRIVATE_KEY").ok())
    {
        let domain = env::var("SSL_DOMAIN").unwrap_or_else(|_| "localhost".to_string());
        certs.push(TlsIdentity::new(domain, certs_pem, key_pem));
    }
    certs
}

fn default_localhost_cert() -> Result<TlsIdentity, PortfuError> {
    let cert = generate_simple_self_signed(vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        "::1".to_string(),
    ])
    .map_err(|e| PortfuError::Internal(format!("failed to generate self-signed cert: {e}")))?;
    Ok(TlsIdentity::new(
        "localhost",
        cert.cert.pem(),
        cert.key_pair.serialize_pem(),
    ))
}

fn load_certs(bytes: &[u8]) -> Result<Vec<CertificateDer<'static>>, PortfuError> {
    CertificateDer::pem_slice_iter(bytes)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| PortfuError::Parsing(format!("Invalid certificate PEM: {error}")))
}

fn load_private_key(bytes: &[u8]) -> Result<PrivateKeyDer<'static>, PortfuError> {
    PrivateKeyDer::from_pem_slice(bytes)
        .map_err(|error| PortfuError::Parsing(format!("Invalid private key PEM: {error}")))
}

#[derive(Debug)]
pub struct NamedClientVerifier {
    mandatory: bool,
    root_hints: Vec<DistinguishedName>,
    verifiers: Vec<(String, Arc<dyn ClientCertVerifier>)>,
}

impl NamedClientVerifier {
    fn build(
        config: &TlsConfig,
        provider: Arc<rustls::crypto::CryptoProvider>,
    ) -> Result<Option<Arc<Self>>, PortfuError> {
        if config.client_auth.presentation == ClientCertificateMode::Disabled {
            return Ok(None);
        }
        if config.client_auth.trust_stores.is_empty() {
            return Err(PortfuError::Internal(
                "Client certificate authentication requires at least one trust store".to_string(),
            ));
        }

        let mut names = HashSet::new();
        let mut root_hints = Vec::new();
        let mut verifiers = Vec::with_capacity(config.client_auth.trust_stores.len());
        for trust_store in &config.client_auth.trust_stores {
            if trust_store.name.trim().is_empty() {
                return Err(PortfuError::Internal(
                    "Client certificate trust store names cannot be empty".to_string(),
                ));
            }
            if !names.insert(trust_store.name.clone()) {
                return Err(PortfuError::Internal(format!(
                    "Duplicate client certificate trust store `{}`",
                    trust_store.name
                )));
            }
            let mut roots = RootCertStore::empty();
            let certs = load_certs(&trust_store.certificates_pem)?;
            if certs.is_empty() {
                return Err(PortfuError::Internal(format!(
                    "Client certificate trust store `{}` contains no certificates",
                    trust_store.name
                )));
            }
            for cert in certs {
                roots.add(cert).map_err(|e| {
                    PortfuError::Internal(format!(
                        "Invalid certificate in trust store `{}`: {e}",
                        trust_store.name
                    ))
                })?;
            }
            root_hints.extend(roots.subjects());
            let verifier =
                WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider.clone())
                    .build()
                    .map_err(|e| {
                        PortfuError::Internal(format!(
                            "Failed to build trust store `{}`: {e}",
                            trust_store.name
                        ))
                    })?;
            verifiers.push((trust_store.name.clone(), verifier));
        }
        Ok(Some(Arc::new(Self {
            mandatory: config.client_auth.presentation == ClientCertificateMode::Required,
            root_hints,
            verifiers,
        })))
    }

    pub fn identity(
        &self,
        peer_certificates: &[CertificateDer<'static>],
    ) -> Option<ClientIdentity> {
        let (leaf, intermediates) = peer_certificates.split_first()?;
        let now = UnixTime::now();
        let verified_by = self
            .verifiers
            .iter()
            .filter_map(|(name, verifier)| {
                verifier
                    .verify_client_cert(leaf, intermediates, now)
                    .ok()
                    .map(|_| name.clone())
            })
            .collect();
        let sha256_fingerprint: [u8; 32] = Sha256::digest(leaf.as_ref()).into();
        Some(ClientIdentity {
            leaf_der: Arc::from(leaf.as_ref()),
            chain_der: intermediates
                .iter()
                .map(|cert| Arc::from(cert.as_ref()))
                .collect::<Vec<_>>()
                .into(),
            sha256_fingerprint,
            verified_by,
        })
    }
}

impl ClientCertVerifier for NamedClientVerifier {
    fn client_auth_mandatory(&self) -> bool {
        self.mandatory
    }

    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &self.root_hints
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        let mut last_error = None;
        for (_, verifier) in &self.verifiers {
            match verifier.verify_client_cert(end_entity, intermediates, now) {
                Ok(verified) => return Ok(verified),
                Err(error) => last_error = Some(error),
            }
        }
        Err(last_error.expect("NamedClientVerifier always contains a verifier"))
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.verifiers[0]
            .1
            .verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.verifiers[0]
            .1
            .verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.verifiers[0].1.supported_verify_schemes()
    }
}

pub fn negotiated_tls_version(version: rustls::ProtocolVersion) -> Option<NegotiatedTlsVersion> {
    match version {
        rustls::ProtocolVersion::TLSv1_2 => Some(NegotiatedTlsVersion::Tls12),
        rustls::ProtocolVersion::TLSv1_3 => Some(NegotiatedTlsVersion::Tls13),
        _ => None,
    }
}

#[derive(Debug, Default)]
pub struct ResolvesServerCertUsingSniWithDefault {
    by_name: HashMap<String, Arc<CertifiedKey>>,
    default: Option<Arc<CertifiedKey>>,
}
impl ResolvesServerCertUsingSniWithDefault {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn add(&mut self, name: &str, ck: CertifiedKey) -> Result<(), PortfuError> {
        let checked_name = DnsName::try_from(name)
            .map_err(|_| PortfuError::Io(Error::new(ErrorKind::InvalidInput, "Bad DNS name")))?;
        let normalized = checked_name.to_lowercase_owned();
        ck.end_entity_cert()
            .and_then(ParsedCertificate::try_from)
            .map_err(|_| PortfuError::Io(Error::new(ErrorKind::InvalidInput, "Bad Entity Cert")))?;
        let key = Arc::new(ck);
        if self.default.is_none() {
            self.default = Some(key.clone());
        }
        self.by_name.insert(normalized.as_ref().to_string(), key);
        Ok(())
    }
}

impl ResolvesServerCert for ResolvesServerCertUsingSniWithDefault {
    fn resolve(&self, client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        if let Some(name) = client_hello.server_name() {
            self.by_name
                .get(&name.to_ascii_lowercase())
                .cloned()
                .or_else(|| self.default.clone())
        } else {
            self.default.clone()
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/server_ssl.rs"]
mod tests;
