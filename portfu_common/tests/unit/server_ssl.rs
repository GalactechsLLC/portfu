use crate::server::config::{
    ClientAuthConfig, ClientCertificateMode, ServerConfig, TlsConfig, TlsIdentity,
    TlsVersionPolicy, TrustStore,
};
use crate::server::ssl::{NamedClientVerifier, load_ssl_certs};
use rcgen::{
    BasicConstraints, Certificate, CertificateParams, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    KeyUsagePurpose,
};
use rustls::crypto::aws_lc_rs::default_provider;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::server::danger::ClientCertVerifier;
use rustls::{ClientConfig, RootCertStore};
use sha2::{Digest, Sha256};
use std::sync::Arc;

fn certificate_authority() -> (Certificate, KeyPair) {
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
    ];
    let ca_key = KeyPair::generate().unwrap();
    let ca = ca_params.self_signed(&ca_key).unwrap();

    (ca, ca_key)
}

fn client_certificate(
    ca: &Certificate,
    ca_key: &KeyPair,
    expired: bool,
) -> (CertificateDer<'static>, Vec<u8>) {
    let mut leaf_params = CertificateParams::new(vec!["client.test".to_string()]).unwrap();
    leaf_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    leaf_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    if expired {
        leaf_params.not_before = rcgen::date_time_ymd(2020, 1, 1);
        leaf_params.not_after = rcgen::date_time_ymd(2021, 1, 1);
    }
    let leaf_key = KeyPair::generate().unwrap();
    let leaf = leaf_params.signed_by(&leaf_key, ca, ca_key).unwrap();
    let leaf_der = CertificateDer::from(leaf.der().to_vec());
    (leaf_der, leaf_key.serialize_der())
}

fn certificate_chain() -> (Certificate, KeyPair, CertificateDer<'static>) {
    let (ca, ca_key) = certificate_authority();
    let (leaf_der, _) = client_certificate(&ca, &ca_key, false);
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

#[tokio::test]
async fn tls_13_policy_accepts_tls_13_and_rejects_tls_12() {
    let generated = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let identity = TlsIdentity::new(
        "localhost",
        generated.cert.pem(),
        generated.key_pair.serialize_pem(),
    );
    let server = ServerConfig {
        tls: Some(TlsConfig::new(identity).versions(TlsVersionPolicy::Tls13Only)),
        ..ServerConfig::default()
    };
    let loaded = load_ssl_certs(&server).expect("TLS config should load");
    let mut roots = RootCertStore::empty();
    roots
        .add(generated.cert.der().clone())
        .expect("test root should load");

    let tls13_client = ClientConfig::builder_with_provider(Arc::new(default_provider()))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_root_certificates(roots.clone())
        .with_no_client_auth();
    assert!(handshake(loaded.server_config.clone(), tls13_client, "localhost").await);

    let tls12_client = ClientConfig::builder_with_provider(Arc::new(default_provider()))
        .with_protocol_versions(&[&rustls::version::TLS12])
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    assert!(!handshake(loaded.server_config, tls12_client, "localhost").await);
}

#[tokio::test]
async fn optional_client_auth_allows_clients_without_certificates() {
    let server_identity =
        rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let (client_ca, _, _) = certificate_chain();
    let tls = TlsConfig::new(TlsIdentity::new(
        "localhost",
        server_identity.cert.pem(),
        server_identity.key_pair.serialize_pem(),
    ))
    .client_auth(ClientAuthConfig {
        presentation: ClientCertificateMode::Optional,
        trust_stores: vec![TrustStore::new("known-clients", client_ca.pem())],
    });
    let loaded = load_ssl_certs(&ServerConfig {
        tls: Some(tls),
        ..ServerConfig::default()
    })
    .unwrap();
    let mut roots = RootCertStore::empty();
    roots.add(server_identity.cert.der().clone()).unwrap();
    let client = ClientConfig::builder_with_provider(Arc::new(default_provider()))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();

    assert!(handshake(loaded.server_config, client, "localhost").await);
}

#[tokio::test]
async fn client_certificate_handshake_matrix() {
    let server_identity =
        rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let (public_ca, public_ca_key) = certificate_authority();
    let (internal_ca, internal_ca_key) = certificate_authority();
    let (unrelated_ca, unrelated_ca_key) = certificate_authority();
    let (public_leaf, public_key) = client_certificate(&public_ca, &public_ca_key, false);
    let (internal_leaf, internal_key) = client_certificate(&internal_ca, &internal_ca_key, false);
    let (unrelated_leaf, unrelated_key) =
        client_certificate(&unrelated_ca, &unrelated_ca_key, false);
    let (expired_leaf, expired_key) = client_certificate(&public_ca, &public_ca_key, true);
    let client_auth = ClientAuthConfig {
        presentation: ClientCertificateMode::Optional,
        trust_stores: vec![
            TrustStore::new("public-clients", public_ca.pem()),
            TrustStore::new("internal-clients", internal_ca.pem()),
        ],
    };
    let tls = TlsConfig::new(TlsIdentity::new(
        "localhost",
        server_identity.cert.pem(),
        server_identity.key_pair.serialize_pem(),
    ))
    .versions(TlsVersionPolicy::Tls13Only)
    .client_auth(client_auth.clone());
    let loaded = load_ssl_certs(&ServerConfig {
        tls: Some(tls),
        ..ServerConfig::default()
    })
    .unwrap();

    assert!(
        handshake(
            loaded.server_config.clone(),
            client_config(
                server_identity.cert.der(),
                Some((public_leaf.clone(), public_key))
            ),
            "localhost",
        )
        .await
    );
    assert_eq!(
        loaded
            .client_verifier
            .as_ref()
            .unwrap()
            .identity(std::slice::from_ref(&public_leaf))
            .unwrap()
            .verified_by,
        ["public-clients"]
    );
    assert!(
        handshake(
            loaded.server_config.clone(),
            client_config(
                server_identity.cert.der(),
                Some((internal_leaf.clone(), internal_key)),
            ),
            "localhost",
        )
        .await
    );
    assert_eq!(
        loaded
            .client_verifier
            .as_ref()
            .unwrap()
            .identity(std::slice::from_ref(&internal_leaf))
            .unwrap()
            .verified_by,
        ["internal-clients"]
    );
    assert!(
        !handshake(
            loaded.server_config.clone(),
            client_config(
                server_identity.cert.der(),
                Some((unrelated_leaf, unrelated_key)),
            ),
            "localhost",
        )
        .await
    );
    assert!(
        !handshake(
            loaded.server_config,
            client_config(
                server_identity.cert.der(),
                Some((expired_leaf, expired_key)),
            ),
            "localhost",
        )
        .await
    );

    let required = TlsConfig::new(TlsIdentity::new(
        "localhost",
        server_identity.cert.pem(),
        server_identity.key_pair.serialize_pem(),
    ))
    .versions(TlsVersionPolicy::Tls13Only)
    .client_auth(ClientAuthConfig {
        presentation: ClientCertificateMode::Required,
        ..client_auth
    });
    let loaded = load_ssl_certs(&ServerConfig {
        tls: Some(required),
        ..ServerConfig::default()
    })
    .unwrap();
    assert!(
        !handshake(
            loaded.server_config,
            client_config(server_identity.cert.der(), None),
            "localhost",
        )
        .await
    );
}

#[tokio::test]
async fn sni_selects_the_matching_non_default_identity() {
    let first = rcgen::generate_simple_self_signed(vec!["first.test".to_string()]).unwrap();
    let second = rcgen::generate_simple_self_signed(vec!["second.test".to_string()]).unwrap();
    let tls = TlsConfig::new(TlsIdentity::new(
        "first.test",
        first.cert.pem(),
        first.key_pair.serialize_pem(),
    ))
    .with_identity(TlsIdentity::new(
        "second.test",
        second.cert.pem(),
        second.key_pair.serialize_pem(),
    ));
    let loaded = load_ssl_certs(&ServerConfig {
        tls: Some(tls),
        ..ServerConfig::default()
    })
    .unwrap();
    let mut roots = RootCertStore::empty();
    roots.add(first.cert.der().clone()).unwrap();
    roots.add(second.cert.der().clone()).unwrap();
    let client = ClientConfig::builder_with_provider(Arc::new(default_provider()))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();

    assert!(handshake(loaded.server_config, client, "second.test").await);
}

#[test]
fn malformed_tls_material_is_rejected_during_configuration() {
    let server = ServerConfig {
        tls: Some(TlsConfig::new(TlsIdentity::new(
            "localhost",
            "not a certificate",
            "not a private key",
        ))),
        ..ServerConfig::default()
    };

    assert!(load_ssl_certs(&server).is_err());
}

async fn handshake(
    server: Arc<rustls::ServerConfig>,
    client: ClientConfig,
    server_name: &'static str,
) -> bool {
    let (client_io, server_io) = tokio::io::duplex(16 * 1024);
    let server = tokio_rustls::TlsAcceptor::from(server).accept(server_io);
    let client = tokio_rustls::TlsConnector::from(Arc::new(client)).connect(
        ServerName::try_from(server_name).unwrap().to_owned(),
        client_io,
    );
    let (server_result, client_result) = tokio::join!(server, client);
    server_result.is_ok() && client_result.is_ok()
}

fn client_config(
    server_certificate: &CertificateDer<'static>,
    identity: Option<(CertificateDer<'static>, Vec<u8>)>,
) -> ClientConfig {
    let mut roots = RootCertStore::empty();
    roots.add(server_certificate.clone()).unwrap();
    let builder = ClientConfig::builder_with_provider(Arc::new(default_provider()))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_root_certificates(roots);
    match identity {
        Some((certificate, private_key)) => builder
            .with_client_auth_cert(
                vec![certificate],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(private_key)),
            )
            .unwrap(),
        None => builder.with_no_client_auth(),
    }
}
