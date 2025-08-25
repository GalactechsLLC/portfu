use crate::server::ServerConfig;
use log::error;
use rand::Rng;
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs1v15::SigningKey;
use rsa::pkcs8::{DecodePrivateKey, EncodePrivateKey, EncodePublicKey};
use rustls::client::danger::HandshakeSignatureValid;
use rustls::crypto::ring::default_provider;
use rustls::crypto::ring::sign::RsaSigningKey;
use rustls::pki_types::{CertificateDer, DnsName, PrivateKeyDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::server::{ClientHello, ParsedCertificate, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use rustls::{DigitallySignedStruct, DistinguishedName, RootCertStore, SignatureScheme};
use rustls_pemfile::{certs, read_one, Item};
use sha2::Sha256;
use std::collections::HashMap;
use std::env;
use std::fmt::Debug;
use std::io::{BufReader, Error, ErrorKind};
use std::ops::Sub;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use x509_cert::builder::{Builder, CertificateBuilder, Profile};
use x509_cert::der::asn1::{Ia5String, UtcTime};
use x509_cert::der::pem::LineEnding;
use x509_cert::der::{DateTime, DecodePem, EncodePem};
use x509_cert::ext::pkix::name::GeneralName;
use x509_cert::ext::pkix::SubjectAltName;
use x509_cert::name::Name;
use x509_cert::serial_number::SerialNumber;
use x509_cert::spki::SubjectPublicKeyInfo;
use x509_cert::time::{Time, Validity};
use x509_cert::Certificate;

pub fn load_ssl_certs(config: &ServerConfig) -> Result<Arc<rustls::ServerConfig>, Error> {
    default_provider().install_default().unwrap_or_default();
    let mut root_cert_store = RootCertStore::empty();
    let mut resolver = ResolvesServerCertUsingSniWithDefault::new();
    let (certs, key, root_certs) = if let Some(ssl_info) = &config.ssl_config {
        (
            load_certs(ssl_info.certs.as_bytes())?,
            load_private_key(ssl_info.key.as_bytes())?,
            load_certs(ssl_info.root_certs.as_bytes())?,
        )
    } else if let (Some(crt), Some(key), Some(domain), Some(name)) = (
        env::var("PRIVATE_CA_CRT").ok(),
        env::var("PRIVATE_CA_KEY").ok(),
        env::var("SSL_DOMAIN").ok(),
        env::var("SSL_CRT_NAME").ok(),
    ) {
        let (cert_bytes, key_bytes) = generate_ca_signed_cert(
            crt.as_bytes(),
            key.as_bytes(),
            &domain,
            Name::from_str(&name).map_err(|e| Error::other(format!("{e:?}")))?,
        )?;
        (
            load_certs(&cert_bytes)?,
            load_private_key(&key_bytes)?,
            load_certs(crt.as_bytes())?,
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
        return Err(Error::new(ErrorKind::InvalidInput, "Invalid SSL Config"));
    };
    for cert in root_certs {
        root_cert_store.add(cert).map_err(|e| {
            Error::new(
                ErrorKind::InvalidInput,
                format!("Invalid Root Cert for Server: {e:?}"),
            )
        })?;
    }
    let name = config
        .ssl_config
        .as_ref()
        .map(|c| c.domain.as_str())
        .unwrap_or("localhost");
    let cer_key = CertifiedKey::new(
        certs,
        Arc::new(RsaSigningKey::new(&key).map_err(|e| {
            Error::new(
                ErrorKind::InvalidInput,
                format!("Private Key is not Valid SigningKey: {e:?}"),
            )
        })?),
    );
    resolver.add(name, cer_key).map_err(|e| {
        Error::new(
            ErrorKind::InvalidInput,
            format!("Failed to add SSL Certs to Resolver: {e:?}"),
        )
    })?;
    match &config.client_ssl_config {
        None => {
            let resolver = Arc::new(resolver);
            Ok(Arc::new(
                rustls::ServerConfig::builder()
                    .with_no_client_auth()
                    .with_cert_resolver(resolver),
            ))
        }
        Some(client_ssl) => {
            let (certs, key, root_certs) = (
                load_certs(client_ssl.certs.as_bytes())?,
                load_private_key(client_ssl.key.as_bytes())?,
                load_certs(client_ssl.root_certs.as_bytes())?,
            );
            for cert in root_certs {
                root_cert_store.add(cert).map_err(|e| {
                    Error::new(
                        ErrorKind::InvalidInput,
                        format!("Invalid Root Cert for Server: {e:?}"),
                    )
                })?;
            }
            let cer_key = CertifiedKey::new(
                certs,
                Arc::new(RsaSigningKey::new(&key).map_err(|e| {
                    Error::new(
                        ErrorKind::InvalidInput,
                        format!("Private Key is not Valid SigningKey: {e:?}"),
                    )
                })?),
            );
            resolver
                .add(client_ssl.domain.as_str(), cer_key)
                .map_err(|e| {
                    Error::new(
                        ErrorKind::InvalidInput,
                        format!("Failed to add SSL Certs to Resolver: {e:?}"),
                    )
                })?;
            let resolver = Arc::new(resolver);
            Ok(Arc::new(
                rustls::ServerConfig::builder()
                    .with_client_cert_verifier(AllowAny::new())
                    .with_cert_resolver(resolver),
            ))
        }
    }
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
            return Ok(item);
        }
    }
    Err(Error::new(ErrorKind::NotFound, "Private Key Not Found"))
}

pub fn generate_ca_signed_cert(
    cert_data: &[u8],
    key_data: &[u8],
    dns_name: &str,
    name: Name,
) -> Result<(Vec<u8>, Vec<u8>), Error> {
    let root_cert = Certificate::from_pem(cert_data).map_err(|e| Error::other(format!("{e:?}")))?;
    let root_key = rsa::RsaPrivateKey::from_pkcs1_pem(&String::from_utf8_lossy(key_data))
        .or_else(|_| rsa::RsaPrivateKey::from_pkcs8_pem(&String::from_utf8_lossy(key_data)))
        .map_err(|e| Error::other(format!("Failed to load Root Key: {e:?}")))?;
    let mut rng = rand::thread_rng();
    let cert_key =
        rsa::RsaPrivateKey::new(&mut rng, 2048).map_err(|e| Error::other(format!("{e:?}")))?;
    let pub_key = cert_key.to_public_key();
    let signing_key: SigningKey<Sha256> = SigningKey::new(root_key);
    let subject_pub_key = SubjectPublicKeyInfo::from_pem(
        pub_key
            .to_public_key_pem(LineEnding::default())
            .map_err(|e| Error::other(format!("{e:?}")))?
            .as_bytes(),
    )
    .map_err(|e| Error::other(format!("{e:?}")))?;
    let mut cert = CertificateBuilder::new(
        Profile::Leaf {
            issuer: root_cert.tbs_certificate.issuer,
            enable_key_agreement: false,
            enable_key_encipherment: false,
        },
        SerialNumber::from(rng.gen::<u32>()),
        Validity {
            not_before: Time::UtcTime(
                UtcTime::from_system_time(SystemTime::now().sub(Duration::from_secs(60 * 60 * 24)))
                    .map_err(|e| Error::other(format!("{e:?}")))?,
            ),
            not_after: Time::UtcTime(
                UtcTime::from_date_time(
                    DateTime::new(2049, 8, 2, 0, 0, 0)
                        .map_err(|e| Error::other(format!("{e:?}")))?,
                )
                .map_err(|e| Error::other(format!("{e:?}")))?,
            ),
        },
        name,
        subject_pub_key,
        &signing_key,
    )
    .map_err(|e| Error::other(format!("{e:?}")))?;
    cert.add_extension(&SubjectAltName(vec![GeneralName::DnsName(
        Ia5String::new(dns_name).map_err(|e| Error::other(format!("{e:?}")))?,
    )]))
    .map_err(|e| Error::other(format!("{e:?}")))?;
    let cert = cert.build().map_err(|e| Error::other(format!("{e:?}")))?;
    Ok((
        cert.to_pem(LineEnding::default())
            .map_err(|e| Error::other(format!("{e:?}")))?
            .as_bytes()
            .to_vec(),
        cert_key
            .to_pkcs8_pem(LineEnding::default())
            .map_err(|e| Error::other(format!("{e:?}")))?
            .as_bytes()
            .to_vec(),
    ))
}

fn handle_item(item: Result<Item, Error>) -> Option<PrivateKeyDer<'static>> {
    match item {
        Ok(Item::Pkcs8Key(key)) => {
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

#[derive(Debug)]
pub struct ResolvesServerCertUsingSniWithDefault {
    by_name: HashMap<String, Arc<CertifiedKey>>,
}
impl ResolvesServerCertUsingSniWithDefault {
    pub fn new() -> Self {
        Self {
            by_name: HashMap::new(),
        }
    }
    pub fn add(&mut self, name: &str, ck: CertifiedKey) -> Result<(), Error> {
        let server_name = {
            let checked_name = DnsName::try_from(name)
                .map_err(|_| Error::new(ErrorKind::InvalidInput, "Bad DNS name"))
                .map(|name| name.to_lowercase_owned())?;
            ServerName::DnsName(checked_name)
        };
        ck.end_entity_cert()
            .and_then(ParsedCertificate::try_from)
            .map_err(|_| Error::new(ErrorKind::InvalidInput, "Bad Entity Cert"))?;
        if let ServerName::DnsName(name) = server_name {
            self.by_name.insert(name.as_ref().to_string(), Arc::new(ck));
        }
        Ok(())
    }
}

impl ResolvesServerCert for ResolvesServerCertUsingSniWithDefault {
    fn resolve(&self, client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        if let Some(name) = client_hello.server_name() {
            self.by_name
                .get(name)
                .cloned()
                .or_else(|| self.by_name.values().next().cloned())
        } else {
            self.by_name.values().next().cloned()
        }
    }
}
