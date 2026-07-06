use crate::error::PortfuError;
use crate::server::config::{ServerConfig, SslConfig};
use rcgen::generate_simple_self_signed;
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs1v15::SigningKey;
use rsa::pkcs8::{DecodePrivateKey, EncodePrivateKey, EncodePublicKey};
use rsa::rand_core::RngCore;
use rustls::client::danger::HandshakeSignatureValid;
use rustls::crypto::aws_lc_rs::default_provider;
use rustls::crypto::aws_lc_rs::sign::any_supported_type;
use rustls::pki_types::{CertificateDer, DnsName, PrivateKeyDer, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::server::{ClientHello, ParsedCertificate, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use rustls::{DigitallySignedStruct, DistinguishedName, RootCertStore, SignatureScheme};
use rustls_pemfile::{Item, certs, read_one};
use sha2::Sha256;
use std::collections::HashMap;
use std::env;
use std::io::{BufReader, Error, ErrorKind};
use std::ops::Sub;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use x509_cert::Certificate;
use x509_cert::builder::{Builder, CertificateBuilder, Profile};
use x509_cert::der::asn1::{Ia5String, UtcTime};
use x509_cert::der::pem::LineEnding;
use x509_cert::der::{DateTime, DecodePem, EncodePem};
use x509_cert::ext::pkix::SubjectAltName;
use x509_cert::ext::pkix::name::GeneralName;
use x509_cert::name::Name;
use x509_cert::serial_number::SerialNumber;
use x509_cert::spki::SubjectPublicKeyInfo;
use x509_cert::time::{Time, Validity};

pub fn load_ssl_certs(config: &ServerConfig) -> Result<Arc<rustls::ServerConfig>, PortfuError> {
    default_provider()
        .install_default()
        .map_err(|e| PortfuError::Internal(format!("failed to install rustls provider: {e:?}")))?;
    let mut root_cert_store = RootCertStore::empty();
    let mut resolver = ResolvesServerCertUsingSniWithDefault::new();

    let mut cert_configs = collect_server_certs(config)?;
    if cert_configs.is_empty() {
        cert_configs.push(default_localhost_cert()?);
    }

    for cert_config in cert_configs {
        for cert in load_certs(cert_config.root_certs.as_bytes())? {
            root_cert_store.add(cert).map_err(|e| {
                PortfuError::Internal(format!("Invalid Root Cert for Server: {e:?}"))
            })?;
        }
        let certs = load_certs(cert_config.certs.as_bytes())?;
        let key = load_private_key(cert_config.key.as_bytes())?;
        let signing_key = any_supported_type(&key)
            .map_err(|e| PortfuError::Internal(format!("Private key is invalid: {e:?}")))?;
        let cert_key = CertifiedKey::new(certs, signing_key);
        resolver.add(cert_config.domain.as_str(), cert_key)?;
    }

    if let Some(client_ssl) = &config.client_ssl_config {
        for cert in load_certs(client_ssl.root_certs.as_bytes())? {
            root_cert_store.add(cert).map_err(|e| {
                PortfuError::Internal(format!("Invalid Root Cert for Client Verification: {e:?}"))
            })?;
        }
        let resolver = Arc::new(resolver);
        Ok(Arc::new(
            rustls::ServerConfig::builder()
                .with_client_cert_verifier(AllowAny::new())
                .with_cert_resolver(resolver),
        ))
    } else {
        let resolver = Arc::new(resolver);
        Ok(Arc::new(
            rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_cert_resolver(resolver),
        ))
    }
}

fn collect_server_certs(config: &ServerConfig) -> Result<Vec<SslConfig>, PortfuError> {
    let mut certs = vec![];
    if let Some(default) = &config.ssl_config {
        certs.push(default.clone());
    }
    certs.extend(config.sni_ssl_configs.iter().cloned());
    if certs.is_empty()
        && let (Some(ca_crt), Some(ca_key)) = (
            env::var("PRIVATE_CA_CRT").ok(),
            env::var("PRIVATE_CA_KEY").ok(),
        )
    {
        let domain = env::var("SSL_DOMAIN").unwrap_or_else(|_| "localhost".to_string());
        let cert_name =
            env::var("SSL_CRT_NAME").unwrap_or_else(|_| "CN=localhost, O=Portfu, C=US".to_string());
        let (cert_bytes, key_bytes) = generate_ca_signed_cert(
            ca_crt.as_bytes(),
            ca_key.as_bytes(),
            domain.as_str(),
            Name::from_str(cert_name.as_str()).map_err(|e| {
                PortfuError::Parsing(format!("Invalid SSL_CRT_NAME value `{cert_name}`: {e:?}"))
            })?,
        )?;
        certs.push(SslConfig {
            domain,
            key: String::from_utf8(key_bytes)
                .map_err(|e| PortfuError::Parsing(format!("Invalid generated key bytes: {e}")))?,
            certs: String::from_utf8(cert_bytes).map_err(|e| {
                PortfuError::Parsing(format!("Invalid generated certificate bytes: {e}"))
            })?,
            root_certs: ca_crt,
        });
    }
    if certs.is_empty()
        && let (Some(certs_pem), Some(key_pem), Some(root_certs_pem)) = (
            env::var("SSL_CERTS").ok(),
            env::var("SSL_PRIVATE_KEY").ok(),
            env::var("SSL_ROOT_CERTS").ok(),
        )
    {
        let domain = env::var("SSL_DOMAIN").unwrap_or_else(|_| "localhost".to_string());
        certs.push(SslConfig {
            domain,
            key: key_pem,
            certs: certs_pem,
            root_certs: root_certs_pem,
        });
    }
    Ok(certs)
}

fn default_localhost_cert() -> Result<SslConfig, PortfuError> {
    let cert = generate_simple_self_signed(vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        "::1".to_string(),
    ])
    .map_err(|e| PortfuError::Internal(format!("failed to generate self-signed cert: {e}")))?;
    Ok(SslConfig {
        domain: "localhost".to_string(),
        key: cert.key_pair.serialize_pem(),
        certs: cert.cert.pem(),
        root_certs: cert.cert.pem(),
    })
}

fn load_certs(bytes: &[u8]) -> Result<Vec<CertificateDer<'static>>, PortfuError> {
    let mut reader = BufReader::new(bytes);
    Ok(certs(&mut reader).flatten().collect())
}

fn load_private_key(bytes: &[u8]) -> Result<PrivateKeyDer<'static>, PortfuError> {
    let mut reader = BufReader::new(bytes);
    for item in std::iter::from_fn(|| read_one(&mut reader).transpose()) {
        if let Some(item) = handle_item(item).map_err(PortfuError::Io)? {
            return Ok(item);
        }
    }
    Err(PortfuError::Io(Error::new(
        ErrorKind::NotFound,
        "Private Key Not Found",
    )))
}

fn generate_ca_signed_cert(
    cert_data: &[u8],
    key_data: &[u8],
    dns_name: &str,
    name: Name,
) -> Result<(Vec<u8>, Vec<u8>), PortfuError> {
    let root_cert = Certificate::from_pem(cert_data)
        .map_err(|e| PortfuError::Internal(format!("Failed to parse PRIVATE_CA_CRT: {e:?}")))?;
    let root_key = rsa::RsaPrivateKey::from_pkcs1_pem(&String::from_utf8_lossy(key_data))
        .or_else(|_| rsa::RsaPrivateKey::from_pkcs8_pem(&String::from_utf8_lossy(key_data)))
        .map_err(|e| PortfuError::Internal(format!("Failed to parse PRIVATE_CA_KEY: {e:?}")))?;
    let mut rng = rsa::rand_core::OsRng;
    let cert_key = rsa::RsaPrivateKey::new(&mut rng, 2048)
        .map_err(|e| PortfuError::Internal(format!("Failed to generate cert key: {e:?}")))?;
    let pub_key = cert_key.to_public_key();
    let signing_key: SigningKey<Sha256> = SigningKey::new(root_key);
    let subject_pub_key = SubjectPublicKeyInfo::from_pem(
        pub_key
            .to_public_key_pem(LineEnding::default())
            .map_err(|e| {
                PortfuError::Internal(format!("Failed to convert generated pub key to PEM: {e:?}"))
            })?
            .as_bytes(),
    )
    .map_err(|e| PortfuError::Internal(format!("Failed to parse generated pub key PEM: {e:?}")))?;
    let mut cert = CertificateBuilder::new(
        Profile::Leaf {
            issuer: root_cert.tbs_certificate.issuer,
            enable_key_agreement: false,
            enable_key_encipherment: false,
        },
        SerialNumber::from(rng.next_u32()),
        Validity {
            not_before: Time::UtcTime(
                UtcTime::from_system_time(SystemTime::now().sub(Duration::from_secs(60 * 60 * 24)))
                    .map_err(|e| {
                        PortfuError::Internal(format!("Failed to build cert not_before: {e:?}"))
                    })?,
            ),
            not_after: Time::UtcTime(
                UtcTime::from_date_time(DateTime::new(2049, 8, 2, 0, 0, 0).map_err(|e| {
                    PortfuError::Internal(format!("Failed to build cert not_after: {e:?}"))
                })?)
                .map_err(|e| {
                    PortfuError::Internal(format!("Failed to build cert not_after utc time: {e:?}"))
                })?,
            ),
        },
        name,
        subject_pub_key,
        &signing_key,
    )
    .map_err(|e| PortfuError::Internal(format!("Failed to build generated certificate: {e:?}")))?;
    cert.add_extension(&SubjectAltName(vec![GeneralName::DnsName(
        Ia5String::new(dns_name)
            .map_err(|e| PortfuError::Internal(format!("Invalid dns name `{dns_name}`: {e:?}")))?,
    )]))
    .map_err(|e| PortfuError::Internal(format!("Failed to add SAN extension: {e:?}")))?;
    let cert = cert
        .build()
        .map_err(|e| PortfuError::Internal(format!("Failed to finalize generated cert: {e:?}")))?;
    Ok((
        cert.to_pem(LineEnding::default())
            .map_err(|e| PortfuError::Internal(format!("Failed to encode generated cert: {e:?}")))?
            .as_bytes()
            .to_vec(),
        cert_key
            .to_pkcs8_pem(LineEnding::default())
            .map_err(|e| {
                PortfuError::Internal(format!("Failed to encode generated private key: {e:?}"))
            })?
            .as_bytes()
            .to_vec(),
    ))
}

fn handle_item(item: Result<Item, Error>) -> Result<Option<PrivateKeyDer<'static>>, Error> {
    Ok(match item? {
        Item::Pkcs8Key(key) => Some(PrivateKeyDer::Pkcs8(key)),
        Item::Pkcs1Key(key) => Some(PrivateKeyDer::Pkcs1(key)),
        Item::Sec1Key(key) => Some(PrivateKeyDer::Sec1(key)),
        _ => None,
    })
}

#[derive(Debug)]
pub struct AllowAny {}
impl AllowAny {
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {})
    }
}
impl ClientCertVerifier for AllowAny {
    fn client_auth_mandatory(&self) -> bool {
        false
    }

    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::ECDSA_NISTP521_SHA512,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::ED25519,
        ]
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
        if self.default.is_none() || normalized.as_ref() == "localhost" {
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
