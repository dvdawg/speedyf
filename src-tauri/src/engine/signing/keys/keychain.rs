//! Identities in the macOS Keychain.
//!
//! The private key stays in the Keychain: signing hands it the message and
//! gets the signature back (`SecKeyCreateSignature`), and macOS asks the user
//! whether SpeedyF may use the key. Every function takes the keychains to
//! search or write; `None` means the user's default ones, and the tests pass
//! a temporary keychain file so they never touch the login keychain.

use super::{
    can_sign_documents, describe, fingerprint, order_chain, CmsSigner, IdentityDto, SignAlg,
};
use crate::errors::{AppError, AppResult};
use security_framework::certificate::SecCertificate;
use security_framework::identity::SecIdentity;
use security_framework::import_export::Pkcs12ImportOptions;
use security_framework::item::{ItemClass, ItemSearchOptions, Limit, Reference, SearchResult};
use security_framework::key::{Algorithm, SecKey};
use security_framework::os::macos::keychain::SecKeychain;
use security_framework::policy::SecPolicy;
use security_framework::trust::SecTrust;

pub(crate) const ID_PREFIX: &str = "keychain:";

fn sf_err(context: &str, e: impl std::fmt::Display) -> AppError {
    AppError::Unsupported(format!("{context}: {e}"))
}

fn identities(keychains: Option<&[SecKeychain]>) -> AppResult<Vec<SecIdentity>> {
    let mut search = ItemSearchOptions::new();
    search
        .class(ItemClass::identity())
        .load_refs(true)
        .limit(Limit::All);
    if let Some(keychains) = keychains {
        search.keychains(keychains);
    }
    let found = match search.search() {
        Ok(found) => found,
        // errSecItemNotFound: an empty keychain.
        Err(e) if e.code() == -25300 => return Ok(Vec::new()),
        Err(e) => return Err(sf_err("cannot search the Keychain", e)),
    };
    Ok(found
        .into_iter()
        .filter_map(|r| match r {
            SearchResult::Ref(Reference::Identity(identity)) => Some(identity),
            _ => None,
        })
        .collect())
}

/// The certificate's chain as far as the Keychain knows it, built offline.
fn chain_for(leaf: &SecCertificate) -> Vec<Vec<u8>> {
    let leaf_der = leaf.to_der();
    let mut others = Vec::new();
    if let Ok(mut trust) =
        SecTrust::create_with_certificates(std::slice::from_ref(leaf), &[SecPolicy::create_x509()])
    {
        // Never fetch missing issuers from the network.
        let _ = trust.set_network_fetch_allowed(false);
        let _ = trust.evaluate_with_error();
        others.extend(trust.chain().iter().skip(1).map(SecCertificate::to_der));
    }
    order_chain(leaf_der, others)
}

/// Identities whose certificates can sign documents.
pub fn list(keychains: Option<&[SecKeychain]>) -> AppResult<Vec<IdentityDto>> {
    let mut out: Vec<IdentityDto> = Vec::new();
    for identity in identities(keychains)? {
        let Ok(cert) = identity.certificate() else {
            continue;
        };
        let der = cert.to_der();
        if !can_sign_documents(&der) {
            continue;
        }
        let id = format!("{ID_PREFIX}{}", fingerprint(&der));
        if out.iter().any(|d| d.id == id) {
            continue;
        }
        if let Ok(dto) = describe(&[der], id, "keychain", false) {
            out.push(dto);
        }
    }
    out.sort_by_key(|d| d.name.to_lowercase());
    Ok(out)
}

/// Import a .p12 into the Keychain, returning the identity it added.
///
/// The Keychain reads older .p12 encryptions more reliably than the AES ones
/// OpenSSL 3 writes by default, so a file it refuses is read here and handed
/// over again in the classic encoding.
pub fn import_p12(
    bytes: &[u8],
    password: &str,
    keychain: Option<&SecKeychain>,
) -> AppResult<IdentityDto> {
    let mut options = Pkcs12ImportOptions::new();
    options.passphrase(password);
    if let Some(kc) = keychain {
        options.keychain(kc.clone());
    }
    let imported = match options.import(bytes) {
        Ok(imported) => imported,
        // errSecAuthFailed / errSecPkcs12VerifyFailure: the password.
        Err(e) if e.code() == -25293 || e.code() == -25264 => {
            return Err(AppError::Unsupported(
                "the password is not right for this file".into(),
            ))
        }
        Err(first) => {
            let Ok((pkcs8, chain)) = super::read_p12(bytes, password) else {
                return Err(sf_err("the Keychain could not import this file", first));
            };
            let leaf = chain.first().cloned().unwrap_or_default();
            return add_inner(&pkcs8, chain, "Imported", keychain)
                .and_then(|_| found(&leaf, keychain));
        }
    };
    let identity = imported
        .into_iter()
        .find_map(|i| i.identity)
        .ok_or_else(|| AppError::Malformed("the .p12 file holds no identity".into()))?;
    let cert = identity
        .certificate()
        .map_err(|e| sf_err("cannot read the imported certificate", e))?;
    found(&cert.to_der(), keychain)
}

/// Describe the identity with this certificate, now in the Keychain.
fn found(cert_der: &[u8], keychain: Option<&SecKeychain>) -> AppResult<IdentityDto> {
    let id = format!("{ID_PREFIX}{}", fingerprint(cert_der));
    if !can_sign_documents(cert_der) {
        return Err(AppError::Unsupported(
            "this certificate is not allowed to sign documents".into(),
        ));
    }
    let _ = keychain;
    describe(&[cert_der.to_vec()], id, "keychain", false)
}

fn add_inner(
    pkcs8: &[u8],
    chain: Vec<Vec<u8>>,
    label: &str,
    keychain: Option<&SecKeychain>,
) -> AppResult<()> {
    use p12_keystore::{
        Certificate, EncryptionAlgorithm, KeyStore, KeyStoreEntry, MacAlgorithm, PrivateKey,
        PrivateKeyChain,
    };
    let key = PrivateKey::from_der(pkcs8).map_err(|e| AppError::Malformed(format!("key: {e}")))?;
    let certs = chain
        .iter()
        .map(|c| Certificate::from_der(c))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| AppError::Malformed(format!("certificate: {e}")))?;
    let mut local_id = [0u8; 20];
    getrandom::fill(&mut local_id)
        .map_err(|e| AppError::Internal(format!("no randomness: {e}")))?;
    let mut store = KeyStore::new();
    store.add_entry(
        label,
        KeyStoreEntry::PrivateKeyChain(PrivateKeyChain::new(local_id.to_vec(), key, certs)),
    );
    let mut secret = [0u8; 18];
    getrandom::fill(&mut secret).map_err(|e| AppError::Internal(format!("no randomness: {e}")))?;
    let password: String = secret.iter().map(|b| format!("{b:02x}")).collect();
    let p12 = store
        .writer(&password)
        .encryption_algorithm(EncryptionAlgorithm::PbeWithShaAnd3KeyTripleDesCbc)
        .mac_algorithm(MacAlgorithm::HmacSha1)
        .write()
        .map_err(|e| AppError::Internal(format!("cannot package the key: {e}")))?;
    let mut options = Pkcs12ImportOptions::new();
    options.passphrase(&password);
    if let Some(kc) = keychain {
        options.keychain(kc.clone());
    }
    options
        .import(&p12)
        .map_err(|e| sf_err("the Keychain refused the new identity", e))?;
    Ok(())
}

/// Put a key SpeedyF made into the Keychain, with its certificate.
pub fn add(
    pkcs8: &[u8],
    chain: Vec<Vec<u8>>,
    label: &str,
    keychain: Option<&SecKeychain>,
) -> AppResult<IdentityDto> {
    let leaf = chain
        .first()
        .cloned()
        .ok_or_else(|| AppError::Malformed("no certificate".into()))?;
    add_inner(pkcs8, chain, label, keychain)?;
    found(&leaf, keychain)
}

/// Signs through the Keychain; the key never leaves it.
pub struct KeychainSigner {
    key: SecKey,
    chain: Vec<Vec<u8>>,
    alg: SignAlg,
}

// SecKey is a CoreFoundation reference: thread-safe to use from one thread
// at a time, which is how a signing runs.
unsafe impl Send for KeychainSigner {}

pub fn signer(id: &str, keychains: Option<&[SecKeychain]>) -> AppResult<KeychainSigner> {
    let want = id.strip_prefix(ID_PREFIX).unwrap_or(id);
    for identity in identities(keychains)? {
        let Ok(cert) = identity.certificate() else {
            continue;
        };
        let der = cert.to_der();
        if fingerprint(&der) != want {
            continue;
        }
        let key = identity
            .private_key()
            .map_err(|e| sf_err("cannot reach the private key", e))?;
        let parsed = super::parse_cert(&der)?;
        let oid = parsed
            .tbs_certificate()
            .subject_public_key_info()
            .algorithm
            .oid;
        let alg = if oid == super::OID_RSA {
            SignAlg::RsaSha256
        } else if oid == super::OID_EC {
            SignAlg::EcdsaSha256
        } else {
            return Err(AppError::Unsupported(
                "this certificate's key type cannot sign PDFs here".into(),
            ));
        };
        return Ok(KeychainSigner {
            key,
            chain: chain_for(&cert),
            alg,
        });
    }
    Err(AppError::Unsupported(
        "that identity is no longer in the Keychain".into(),
    ))
}

impl CmsSigner for KeychainSigner {
    fn chain(&self) -> &[Vec<u8>] {
        &self.chain
    }

    fn alg(&self) -> SignAlg {
        self.alg
    }

    fn sign(&self, message: &[u8]) -> AppResult<Vec<u8>> {
        let algorithm = match self.alg {
            SignAlg::RsaSha256 => Algorithm::RSASignatureMessagePKCS1v15SHA256,
            SignAlg::EcdsaSha256 => Algorithm::ECDSASignatureMessageX962SHA256,
        };
        self.key
            .create_signature(algorithm, message)
            .map_err(|e| AppError::Unsupported(format!("the Keychain did not sign: {e}")))
    }
}
