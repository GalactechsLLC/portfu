use crate::error::PortfuError;
use crate::server::config::{
    ClientCertificateMode, ServerConfig, TlsConfig, TlsIdentity, TlsVersionPolicy,
};
use crate::server::connection::{ClientIdentity, NegotiatedTlsVersion};
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
use rustls::server::{ClientHello, ParsedCertificate, ResolvesServerCert, WebPkiClientVerifier};
use rustls::sign::CertifiedKey;
use rustls::{DigitallySignedStruct, DistinguishedName, RootCertStore, SignatureScheme};
use rustls_pemfile::{Item, certs, read_one};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
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

pub struct LoadedTlsConfig {
    pub server_config: Arc<rustls::ServerConfig>,
    pub client_verifier: Option<Arc<NamedClientVerifier>>,
}

pub fn load_ssl_certs(config: &ServerConfig) -> Result<LoadedTlsConfig, PortfuError> {
    let provider = Arc::new(default_provider());
    let mut resolver = ResolvesServerCertUsingSniWithDefault::new();
    let tls = config.tls.clone().unwrap_or_default();

    let mut cert_configs = collect_server_certs(&tls)?;
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

fn collect_server_certs(config: &TlsConfig) -> Result<Vec<TlsIdentity>, PortfuError> {
    let mut certs = config.identities.clone();
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
        certs.push(TlsIdentity::new(domain, cert_bytes, key_bytes));
    }
    if certs.is_empty()
        && let (Some(certs_pem), Some(key_pem)) =
            (env::var("SSL_CERTS").ok(), env::var("SSL_PRIVATE_KEY").ok())
    {
        let domain = env::var("SSL_DOMAIN").unwrap_or_else(|_| "localhost".to_string());
        certs.push(TlsIdentity::new(domain, certs_pem, key_pem));
    }
    Ok(certs)
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
mod tests {
    use super::NamedClientVerifier;
    use crate::server::config::{ClientAuthConfig, ClientCertificateMode, TlsConfig, TrustStore};
    use rcgen::{
        BasicConstraints, Certificate, CertificateParams, ExtendedKeyUsagePurpose, IsCa, KeyPair,
        KeyUsagePurpose,
    };
    use rustls::crypto::aws_lc_rs::default_provider;
    use rustls::pki_types::{CertificateDer, UnixTime};
    use rustls::server::danger::ClientCertVerifier;
    use sha2::{Digest, Sha256};
    use std::sync::Arc;

    fn certificate_chain() -> (Certificate, KeyPair, CertificateDer<'static>) {
        let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params.key_usages = vec![
            KeyUsagePurpose::DigitalSignature,
            KeyUsagePurpose::KeyCertSign,
            KeyUsagePurpose::CrlSign,
        ];
        let ca_key = KeyPair::generate().unwrap();
        let ca = ca_params.self_signed(&ca_key).unwrap();

        let mut leaf_params = CertificateParams::new(vec!["client.test".to_string()]).unwrap();
        leaf_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        leaf_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        let leaf_key = KeyPair::generate().unwrap();
        let leaf = leaf_params.signed_by(&leaf_key, &ca, &ca_key).unwrap();
        let leaf_der = CertificateDer::from(leaf.der().to_vec());
        (ca, ca_key, leaf_der)
    }

    #[test]
    fn named_verifier_classifies_and_fingerprints_client_certificates() {
        let (public_ca, _, public_leaf) = certificate_chain();
        let (private_ca, _, _) = certificate_chain();
        let config = TlsConfig {
            client_auth: ClientAuthConfig {
                presentation: ClientCertificateMode::Optional,
                trust_stores: vec![
                    TrustStore::new("public-clients", public_ca.pem()),
                    TrustStore::new("internal-clients", private_ca.pem()),
                ],
            },
            ..TlsConfig::default()
        };
        let verifier = NamedClientVerifier::build(&config, Arc::new(default_provider()))
            .unwrap()
            .unwrap();

        assert!(!verifier.client_auth_mandatory());
        verifier
            .verify_client_cert(&public_leaf, &[], UnixTime::now())
            .expect("public certificate should verify");
        let identity = verifier
            .identity(std::slice::from_ref(&public_leaf))
            .unwrap();
        assert_eq!(identity.verified_by, vec!["public-clients"]);
        assert_eq!(
            identity.sha256_fingerprint,
            <[u8; 32]>::from(Sha256::digest(public_leaf.as_ref()))
        );
        assert!(identity.chain_der.is_empty());
    }

    #[test]
    fn named_verifier_rejects_certificates_outside_all_stores() {
        let (trusted_ca, _, _) = certificate_chain();
        let (_, _, unrelated_leaf) = certificate_chain();
        let config = TlsConfig {
            client_auth: ClientAuthConfig {
                presentation: ClientCertificateMode::Required,
                trust_stores: vec![TrustStore::new("trusted", trusted_ca.pem())],
            },
            ..TlsConfig::default()
        };
        let verifier = NamedClientVerifier::build(&config, Arc::new(default_provider()))
            .unwrap()
            .unwrap();

        assert!(verifier.client_auth_mandatory());
        assert!(
            verifier
                .verify_client_cert(&unrelated_leaf, &[], UnixTime::now())
                .is_err()
        );
    }
}
