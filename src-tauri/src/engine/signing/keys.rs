//! Signing identities: a certificate and the private key that goes with it.
//!
//! Three sources, all offline:
//! - the macOS Keychain (keys/keychain.rs, macOS only): every identity in
//!   the user's keychains whose certificate may sign documents. The private
//!   key never leaves the Keychain; signing goes through
//!   `SecKeyCreateSignature`, and macOS asks the user to allow it.
//! - a .p12 the user imports: into the Keychain on macOS, elsewhere into
//!   SpeedyF's own store (keys/filestore.rs), the key encrypted under a
//!   passphrase the user chooses.
//! - a self-signed identity SpeedyF makes from a name, an email address and
//!   an organisation, stored the same way as an import.
//!
//! Identities are personal: they live in the Keychain or the app data
//! directory, never in the library and never in anything synced.

mod filestore;
#[cfg(target_os = "macos")]
mod keychain;
#[cfg(test)]
mod tests;

use crate::errors::{AppError, AppResult};
use der::{Decode, Encode};
use serde::Serialize;
use sha2_11::{Digest, Sha256};
use std::path::Path;
use x509_cert::ext::pkix::name::GeneralName;
use x509_cert::ext::pkix::{BasicConstraints, ExtendedKeyUsage, KeyUsage, SubjectAltName};
use x509_cert::Certificate;

pub use filestore::FileStore;

/// One identity, as the signing dialog lists it.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IdentityDto {
    pub id: String,
    /// "keychain" or "file".
    pub store: &'static str,
    pub name: String,
    pub email: String,
    pub organization: String,
    /// The issuer's common name (or its organisation).
    pub issuer: String,
    /// Validity, in seconds since the Unix epoch.
    pub not_before: i64,
    pub not_after: i64,
    pub self_signed: bool,
    /// Signing asks for the passphrase the key was stored under.
    pub needs_passphrase: bool,
}

/// Where a new identity goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreKind {
    Keychain,
    File,
}

impl StoreKind {
    pub fn parse(s: &str) -> AppResult<StoreKind> {
        match s {
            "keychain" => Ok(StoreKind::Keychain),
            "file" => Ok(StoreKind::File),
            other => Err(AppError::Unsupported(format!(
                "unknown identity store {other}"
            ))),
        }
    }
}

/// The signature scheme a key signs with. Both hash with SHA-256.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignAlg {
    /// RSASSA-PKCS1-v1_5.
    RsaSha256,
    /// ECDSA, the signature DER-encoded (X9.62).
    EcdsaSha256,
}

/// What the CMS builder needs from an identity.
pub trait CmsSigner {
    /// DER certificates, the signer's own first.
    fn chain(&self) -> &[Vec<u8>];
    fn alg(&self) -> SignAlg;
    /// Sign `message`, hashing it with SHA-256 first.
    fn sign(&self, message: &[u8]) -> AppResult<Vec<u8>>;
}

pub type BoxedSigner = Box<dyn CmsSigner + Send>;

// ---------------------------------------------------------------------------
// A key held in memory (the file store, and tests)
// ---------------------------------------------------------------------------

enum SoftKey {
    Rsa(Box<rsa::pkcs1v15::SigningKey<Sha256>>),
    P256(p256::ecdsa::SigningKey),
}

/// A private key in memory, decrypted from the file store for one signing.
pub struct SoftwareSigner {
    chain: Vec<Vec<u8>>,
    key: SoftKey,
}

const OID_RSA: der::asn1::ObjectIdentifier =
    der::asn1::ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.1");
const OID_EC: der::asn1::ObjectIdentifier =
    der::asn1::ObjectIdentifier::new_unwrap("1.2.840.10045.2.1");

impl SoftwareSigner {
    /// A PKCS#8 key and its certificate chain (the key's own first). Refuses a
    /// key that does not belong to the first certificate.
    pub fn from_pkcs8(pkcs8: &[u8], chain: Vec<Vec<u8>>) -> AppResult<SoftwareSigner> {
        use pkcs8::{DecodePrivateKey, EncodePublicKey};
        let info = pkcs8::PrivateKeyInfoRef::from_der(pkcs8)
            .map_err(|e| AppError::Malformed(format!("private key: {e}")))?;
        let (key, public) = if info.algorithm.oid == OID_RSA {
            let private = rsa::PrivateKey::from_pkcs8_der(pkcs8)
                .map_err(|e| AppError::Malformed(format!("RSA key: {e}")))?;
            let public = private
                .to_public_key()
                .to_public_key_der()
                .map_err(|e| AppError::Malformed(format!("RSA key: {e}")))?;
            (
                SoftKey::Rsa(Box::new(rsa::pkcs1v15::SigningKey::<Sha256>::new(private))),
                public,
            )
        } else if info.algorithm.oid == OID_EC {
            let secret = p256::SecretKey::from_pkcs8_der(pkcs8).map_err(|_| {
                AppError::Unsupported("only RSA and P-256 keys can be stored by SpeedyF".into())
            })?;
            let public = secret
                .public_key()
                .to_public_key_der()
                .map_err(|e| AppError::Malformed(format!("EC key: {e}")))?;
            (SoftKey::P256(p256::ecdsa::SigningKey::from(secret)), public)
        } else {
            return Err(AppError::Unsupported(
                "only RSA and P-256 keys can be stored by SpeedyF".into(),
            ));
        };
        let leaf = chain
            .first()
            .ok_or_else(|| AppError::Malformed("no certificate for the key".into()))?;
        if spki_der(leaf)? != public.as_bytes() {
            return Err(AppError::Malformed(
                "the private key does not belong to the certificate".into(),
            ));
        }
        Ok(SoftwareSigner { chain, key })
    }
}

impl CmsSigner for SoftwareSigner {
    fn chain(&self) -> &[Vec<u8>] {
        &self.chain
    }

    fn alg(&self) -> SignAlg {
        match self.key {
            SoftKey::Rsa(_) => SignAlg::RsaSha256,
            SoftKey::P256(_) => SignAlg::EcdsaSha256,
        }
    }

    fn sign(&self, message: &[u8]) -> AppResult<Vec<u8>> {
        use rsa::signature::{SignatureEncoding, Signer};
        match &self.key {
            SoftKey::Rsa(key) => {
                let sig: rsa::pkcs1v15::Signature = key
                    .try_sign(message)
                    .map_err(|e| AppError::Internal(format!("RSA signing failed: {e}")))?;
                Ok(sig.to_vec())
            }
            SoftKey::P256(key) => {
                let sig: p256::ecdsa::Signature = key
                    .try_sign(message)
                    .map_err(|e| AppError::Internal(format!("ECDSA signing failed: {e}")))?;
                Ok(sig.to_der().as_bytes().to_vec())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Certificates
// ---------------------------------------------------------------------------

pub(crate) fn parse_cert(der: &[u8]) -> AppResult<Certificate> {
    Certificate::from_der(der).map_err(|e| AppError::Malformed(format!("certificate: {e}")))
}

/// The certificate's SubjectPublicKeyInfo, DER.
fn spki_der(cert: &[u8]) -> AppResult<Vec<u8>> {
    parse_cert(cert)?
        .tbs_certificate()
        .subject_public_key_info()
        .to_der()
        .map_err(|e| AppError::Malformed(format!("certificate key: {e}")))
}

/// A stable id for a certificate: its SHA-256, hex.
pub(crate) fn fingerprint(cert: &[u8]) -> String {
    Sha256::digest(cert)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn name_part(name: &x509_cert::name::Name, part: &str) -> String {
    let value = match part {
        "cn" => name
            .common_name()
            .ok()
            .flatten()
            .map(|s| s.value().into_owned()),
        "o" => name
            .organization()
            .ok()
            .flatten()
            .map(|s| s.value().into_owned()),
        _ => name.email_address().ok().flatten().map(|s| s.to_string()),
    };
    value.unwrap_or_default()
}

/// How the dialog describes a certificate.
pub(crate) fn describe(
    chain: &[Vec<u8>],
    id: String,
    store: &'static str,
    needs_passphrase: bool,
) -> AppResult<IdentityDto> {
    let leaf = chain
        .first()
        .ok_or_else(|| AppError::Malformed("empty certificate chain".into()))?;
    let cert = parse_cert(leaf)?;
    let tbs = cert.tbs_certificate();
    let subject = tbs.subject();
    let issuer = tbs.issuer();
    let mut email = name_part(subject, "email");
    if email.is_empty() {
        if let Ok(Some((_, SubjectAltName(names)))) = tbs.get_extension::<SubjectAltName>() {
            email = names
                .iter()
                .find_map(|n| match n {
                    GeneralName::Rfc822Name(s) => Some(s.to_string()),
                    _ => None,
                })
                .unwrap_or_default();
        }
    }
    let mut name = name_part(subject, "cn");
    if name.is_empty() {
        name = if email.is_empty() {
            "Unnamed certificate".into()
        } else {
            email.clone()
        };
    }
    let mut issuer_name = name_part(issuer, "cn");
    if issuer_name.is_empty() {
        issuer_name = name_part(issuer, "o");
    }
    let validity = tbs.validity();
    Ok(IdentityDto {
        id,
        store,
        name,
        email,
        organization: name_part(subject, "o"),
        issuer: issuer_name,
        not_before: validity.not_before.to_unix_duration().as_secs() as i64,
        not_after: validity.not_after.to_unix_duration().as_secs() as i64,
        self_signed: subject == issuer,
        needs_passphrase,
    })
}

/// Extended key usages that allow signing documents. A certificate without
/// the extension may be used for anything.
const SIGNING_EKUS: [&str; 6] = [
    "2.5.29.37.0",             // anyExtendedKeyUsage
    "1.3.6.1.5.5.7.3.4",       // emailProtection
    "1.3.6.1.5.5.7.3.2",       // clientAuth (personal certificates, smart cards)
    "1.3.6.1.5.5.7.3.36",      // id-kp-documentSigning
    "1.2.840.113583.1.1.5",    // Adobe Authentic Documents Trust
    "1.3.6.1.4.1.311.10.3.12", // Microsoft document signing
];

/// Whether a certificate may sign a document: not a CA, a key usage that
/// includes signing, and no extended key usage that rules documents out
/// (a code-signing or TLS server certificate, for instance).
pub(crate) fn can_sign_documents(cert_der: &[u8]) -> bool {
    let Ok(cert) = parse_cert(cert_der) else {
        return false;
    };
    let tbs = cert.tbs_certificate();
    if let Ok(Some((_, bc))) = tbs.get_extension::<BasicConstraints>() {
        if bc.ca {
            return false;
        }
    }
    if let Ok(Some((_, ku))) = tbs.get_extension::<KeyUsage>() {
        if !ku.digital_signature() && !ku.non_repudiation() {
            return false;
        }
    }
    match tbs.get_extension::<ExtendedKeyUsage>() {
        Ok(Some((_, ExtendedKeyUsage(oids)))) => oids
            .iter()
            .any(|oid| SIGNING_EKUS.contains(&oid.to_string().as_str())),
        Ok(None) => true,
        Err(_) => false,
    }
}

/// Order a bag of certificates into a chain starting at `leaf`: each next
/// one is the issuer of the one before. Certificates that are not part of
/// the leaf's chain are dropped.
pub(crate) fn order_chain(leaf: Vec<u8>, others: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
    let mut chain = vec![leaf];
    let mut rest = others;
    while let Some(last) = chain.last().and_then(|c| parse_cert(c).ok()) {
        let tbs = last.tbs_certificate();
        if tbs.subject() == tbs.issuer() {
            break;
        }
        let issuer = tbs.issuer().clone();
        let Some(at) = rest.iter().position(|c| {
            parse_cert(c)
                .map(|p| *p.tbs_certificate().subject() == issuer)
                .unwrap_or(false)
        }) else {
            break;
        };
        let next = rest.remove(at);
        if chain.contains(&next) {
            break;
        }
        chain.push(next);
    }
    chain
}

/// Days a self-signed identity is valid for.
const SELF_SIGNED_DAYS: i64 = 5 * 365;

/// A new P-256 key and a self-signed certificate for it, for signing
/// documents: (PKCS#8 key, certificate), both DER.
pub fn make_self_signed(name: &str, email: &str, org: &str) -> AppResult<(Vec<u8>, Vec<u8>)> {
    use rcgen::{
        CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, KeyPair,
        KeyUsagePurpose, SanType, SerialNumber,
    };
    let name = name.trim();
    let email = email.trim();
    let org = org.trim();
    if name.is_empty() {
        return Err(AppError::Unsupported("a name is required".into()));
    }
    let err = |e: rcgen::Error| AppError::Internal(format!("cannot make the certificate: {e}"));
    let key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).map_err(err)?;
    let mut params = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, name);
    if !org.is_empty() {
        dn.push(DnType::OrganizationName, org);
    }
    params.distinguished_name = dn;
    if !email.is_empty() {
        let ia5 = email
            .to_string()
            .try_into()
            .map_err(|_| AppError::Unsupported("the email address must be plain ASCII".into()))?;
        params.subject_alt_names = vec![SanType::Rfc822Name(ia5)];
    }
    params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::ContentCommitment,
    ];
    params.extended_key_usages = vec![
        ExtendedKeyUsagePurpose::EmailProtection,
        ExtendedKeyUsagePurpose::Other(vec![1, 3, 6, 1, 4, 1, 311, 10, 3, 12]),
        ExtendedKeyUsagePurpose::Other(vec![1, 2, 840, 113583, 1, 1, 5]),
    ];
    let mut serial = [0u8; 16];
    getrandom::fill(&mut serial).map_err(|e| AppError::Internal(format!("no randomness: {e}")))?;
    serial[0] &= 0x7f;
    params.serial_number = Some(SerialNumber::from_slice(&serial));
    let now = rcgen::date_time_ymd(1970, 1, 1)
        + std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
    params.not_before = now - std::time::Duration::from_secs(60);
    params.not_after = now + std::time::Duration::from_secs(SELF_SIGNED_DAYS as u64 * 86_400);
    let cert = params.self_signed(&key).map_err(err)?;
    Ok((key.serialize_der(), cert.der().to_vec()))
}

// ---------------------------------------------------------------------------
// The commands' entry points
// ---------------------------------------------------------------------------

/// Every identity that can sign: the Keychain's (macOS) and SpeedyF's own.
pub fn list(dir: &Path) -> AppResult<Vec<IdentityDto>> {
    let mut out = Vec::new();
    #[cfg(target_os = "macos")]
    match keychain::list(None) {
        Ok(found) => out.extend(found),
        Err(e) => log::warn!("cannot list Keychain identities: {e}"),
    }
    out.extend(FileStore::new(dir).list()?);
    Ok(out)
}

/// Import a .p12/.pfx. `passphrase` is the file store's; the Keychain does
/// not take one.
pub fn import_p12(
    dir: &Path,
    store: StoreKind,
    bytes: &[u8],
    password: &str,
    passphrase: Option<&str>,
) -> AppResult<IdentityDto> {
    match store {
        StoreKind::Keychain => {
            #[cfg(target_os = "macos")]
            {
                keychain::import_p12(bytes, password, None)
            }
            #[cfg(not(target_os = "macos"))]
            {
                let _ = (bytes, password);
                Err(no_keychain())
            }
        }
        StoreKind::File => {
            let (pkcs8, chain) = read_p12(bytes, password)?;
            // The same rule the Keychain path applies: a CA or a TLS server
            // certificate is not a signing identity.
            if !chain.first().is_some_and(|leaf| can_sign_documents(leaf)) {
                return Err(AppError::Unsupported(
                    "this certificate is not allowed to sign documents".into(),
                ));
            }
            FileStore::new(dir).add(&pkcs8, chain, require(passphrase)?)
        }
    }
}

/// Make a self-signed identity and store it.
pub fn create_self_signed(
    dir: &Path,
    store: StoreKind,
    name: &str,
    email: &str,
    org: &str,
    passphrase: Option<&str>,
) -> AppResult<IdentityDto> {
    let (pkcs8, cert) = make_self_signed(name, email, org)?;
    match store {
        StoreKind::Keychain => {
            #[cfg(target_os = "macos")]
            {
                keychain::add(&pkcs8, vec![cert], name, None)
            }
            #[cfg(not(target_os = "macos"))]
            {
                let _ = (pkcs8, cert);
                Err(no_keychain())
            }
        }
        StoreKind::File => FileStore::new(dir).add(&pkcs8, vec![cert], require(passphrase)?),
    }
}

/// The signer for an identity `list` returned.
pub fn signer(dir: &Path, id: &str, passphrase: Option<&str>) -> AppResult<BoxedSigner> {
    if let Some(rest) = id.strip_prefix(filestore::ID_PREFIX) {
        let _ = rest;
        let signer = FileStore::new(dir).signer(id, require(passphrase)?)?;
        return Ok(Box::new(signer));
    }
    #[cfg(target_os = "macos")]
    if id.starts_with(keychain::ID_PREFIX) {
        return Ok(Box::new(keychain::signer(id, None)?));
    }
    Err(AppError::Unsupported(
        "that signing identity is no longer available".into(),
    ))
}

/// Forget an identity SpeedyF stored itself. Keychain identities are the
/// user's to manage in Keychain Access.
pub fn remove(dir: &Path, id: &str) -> AppResult<()> {
    if id.starts_with(filestore::ID_PREFIX) {
        return FileStore::new(dir).remove(id);
    }
    Err(AppError::Unsupported(
        "remove Keychain identities in Keychain Access".into(),
    ))
}

fn require(passphrase: Option<&str>) -> AppResult<&str> {
    match passphrase {
        Some(p) if !p.is_empty() => Ok(p),
        _ => Err(AppError::Unsupported("a passphrase is required".into())),
    }
}

#[cfg(not(target_os = "macos"))]
fn no_keychain() -> AppError {
    AppError::Unsupported("there is no Keychain on this system".into())
}

/// The key and chain inside a .p12: (PKCS#8 key, chain with the key's
/// certificate first).
pub(crate) fn read_p12(bytes: &[u8], password: &str) -> AppResult<(Vec<u8>, Vec<Vec<u8>>)> {
    use p12_keystore::{KeyStore, KeyStoreEntry, Pkcs12ImportPolicy};
    let store = KeyStore::from_pkcs12(bytes, password, Pkcs12ImportPolicy::Relaxed).map_err(
        |e| match e {
            p12_keystore::error::Error::MacError(_) => {
                AppError::Unsupported("the password is not right for this file".into())
            }
            other => AppError::Malformed(format!("cannot read this .p12 file: {other}")),
        },
    )?;
    let (_, chain) = store
        .private_key_chain()
        .ok_or_else(|| AppError::Malformed("the .p12 file holds no private key".into()))?;
    let pkcs8 = chain.key().as_der().to_vec();
    let mut certs: Vec<Vec<u8>> = chain.certs().iter().map(|c| c.as_der().to_vec()).collect();
    for (_, entry) in store.entries() {
        if let KeyStoreEntry::Certificate(c) = entry {
            certs.push(c.as_der().to_vec());
        }
    }
    // Put the key's own certificate first, whatever order the file used.
    let public = key_spki(&pkcs8)?;
    let at = certs
        .iter()
        .position(|c| spki_der(c).map(|s| s == public).unwrap_or(false))
        .ok_or_else(|| {
            AppError::Malformed("the .p12 file has no certificate for its key".into())
        })?;
    let leaf = certs.remove(at);
    Ok((pkcs8.clone(), order_chain(leaf, certs)))
}

/// The public key of a PKCS#8 private key, as SubjectPublicKeyInfo DER.
fn key_spki(pkcs8: &[u8]) -> AppResult<Vec<u8>> {
    use pkcs8::{DecodePrivateKey, EncodePublicKey};
    let info = pkcs8::PrivateKeyInfoRef::from_der(pkcs8)
        .map_err(|e| AppError::Malformed(format!("private key: {e}")))?;
    let der = if info.algorithm.oid == OID_RSA {
        rsa::PrivateKey::from_pkcs8_der(pkcs8)
            .map_err(|e| AppError::Malformed(format!("RSA key: {e}")))?
            .to_public_key()
            .to_public_key_der()
    } else if info.algorithm.oid == OID_EC {
        p256::SecretKey::from_pkcs8_der(pkcs8)
            .map_err(|_| AppError::Unsupported("only RSA and P-256 keys are supported".into()))?
            .public_key()
            .to_public_key_der()
    } else {
        return Err(AppError::Unsupported(
            "only RSA and P-256 keys are supported".into(),
        ));
    };
    der.map(|d| d.as_bytes().to_vec())
        .map_err(|e| AppError::Malformed(format!("public key: {e}")))
}
