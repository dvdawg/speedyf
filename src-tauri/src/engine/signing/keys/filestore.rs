//! SpeedyF's own identity store, for systems without a Keychain (and for
//! anyone who asks for it): `<app data>/certificates/identities.json`.
//!
//! Certificates are public and stored as they are, so the list shows without
//! a passphrase. Each private key is PKCS#8 encrypted with ChaCha20-Poly1305
//! under a key derived from the user's passphrase with Argon2id; nothing
//! about the passphrase itself is stored.

use super::{describe, IdentityDto, SoftwareSigner};
use crate::errors::{AppError, AppResult};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub(crate) const ID_PREFIX: &str = "file:";
const FILE: &str = "identities.json";

/// Argon2id cost: the OWASP baseline (19 MiB, two passes).
const ARGON_M_KIB: u32 = 19 * 1024;
const ARGON_T: u32 = 2;
const ARGON_P: u32 = 1;

#[derive(Serialize, Deserialize, Default)]
struct StoreFile {
    v: u32,
    items: Vec<StoredIdentity>,
}

#[derive(Serialize, Deserialize, Clone)]
struct StoredIdentity {
    id: String,
    /// base64 DER certificates, the key's own first
    chain: Vec<String>,
    kdf: Kdf,
    /// base64, 12 bytes
    nonce: String,
    /// base64 ciphertext of the PKCS#8 key, tag appended
    key: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct Kdf {
    alg: String,
    m: u32,
    t: u32,
    p: u32,
    salt: String,
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn unb64(s: &str) -> AppResult<Vec<u8>> {
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .map_err(|e| AppError::Malformed(format!("identity store: {e}")))
}

fn random<const N: usize>() -> AppResult<[u8; N]> {
    let mut out = [0u8; N];
    getrandom::fill(&mut out).map_err(|e| AppError::Internal(format!("no randomness: {e}")))?;
    Ok(out)
}

fn derive(passphrase: &str, kdf: &Kdf) -> AppResult<[u8; 32]> {
    if kdf.alg != "argon2id" {
        return Err(AppError::Unsupported(format!(
            "unknown key derivation {}",
            kdf.alg
        )));
    }
    let params = argon2::Params::new(kdf.m, kdf.t, kdf.p, Some(32))
        .map_err(|e| AppError::Malformed(format!("identity store: {e}")))?;
    let argon = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    let mut out = [0u8; 32];
    argon
        .hash_password_into(passphrase.as_bytes(), &unb64(&kdf.salt)?, &mut out)
        .map_err(|e| AppError::Internal(format!("key derivation failed: {e}")))?;
    Ok(out)
}

fn cipher(key: &[u8; 32]) -> chacha20poly1305::ChaCha20Poly1305 {
    use chacha20poly1305::KeyInit;
    chacha20poly1305::ChaCha20Poly1305::new(key.into())
}

pub struct FileStore {
    dir: PathBuf,
}

impl FileStore {
    pub fn new(dir: &Path) -> FileStore {
        FileStore {
            dir: dir.to_path_buf(),
        }
    }

    fn read(&self) -> AppResult<StoreFile> {
        match std::fs::read_to_string(self.dir.join(FILE)) {
            Ok(text) => serde_json::from_str(&text)
                .map_err(|e| AppError::Malformed(format!("identity store: {e}"))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(StoreFile {
                v: 1,
                items: Vec::new(),
            }),
            Err(e) => Err(AppError::Io(format!("cannot read identities: {e}"))),
        }
    }

    /// Atomic: a temp sibling, synced, renamed over the old file.
    fn write(&self, file: &StoreFile) -> AppResult<()> {
        std::fs::create_dir_all(&self.dir)?;
        let json = serde_json::to_string_pretty(file)
            .map_err(|e| AppError::Internal(format!("identity store: {e}")))?;
        let mut temp = tempfile::Builder::new()
            .prefix(".identities-")
            .suffix(".json")
            .tempfile_in(&self.dir)?;
        std::io::Write::write_all(&mut temp, json.as_bytes())?;
        temp.as_file().sync_all()?;
        temp.persist(self.dir.join(FILE))
            .map_err(|e| AppError::Io(format!("cannot save identities: {e}")))?;
        Ok(())
    }

    fn chain_of(item: &StoredIdentity) -> AppResult<Vec<Vec<u8>>> {
        item.chain.iter().map(|c| unb64(c)).collect()
    }

    pub fn list(&self) -> AppResult<Vec<IdentityDto>> {
        let file = self.read()?;
        let mut out = Vec::new();
        for item in &file.items {
            match Self::chain_of(item)
                .and_then(|chain| describe(&chain, item.id.clone(), "file", true))
            {
                Ok(dto) => out.push(dto),
                Err(e) => log::warn!("skipping stored identity {}: {e}", item.id),
            }
        }
        Ok(out)
    }

    /// Store a key under `passphrase`. Checks the key signs for the chain's
    /// first certificate before anything is written.
    pub fn add(
        &self,
        pkcs8: &[u8],
        chain: Vec<Vec<u8>>,
        passphrase: &str,
    ) -> AppResult<IdentityDto> {
        use chacha20poly1305::aead::Aead;
        SoftwareSigner::from_pkcs8(pkcs8, chain.clone())?;
        let leaf = chain
            .first()
            .ok_or_else(|| AppError::Malformed("no certificate".into()))?;
        let id = format!("{ID_PREFIX}{}", &super::fingerprint(leaf)[..32]);
        let mut file = self.read()?;
        if file.items.iter().any(|i| i.id == id) {
            return Err(AppError::Unsupported(
                "that identity is already stored".into(),
            ));
        }
        let kdf = Kdf {
            alg: "argon2id".into(),
            m: ARGON_M_KIB,
            t: ARGON_T,
            p: ARGON_P,
            salt: b64(&random::<16>()?),
        };
        let key = derive(passphrase, &kdf)?;
        let nonce = random::<12>()?;
        let sealed = cipher(&key)
            .encrypt((&nonce).into(), pkcs8)
            .map_err(|_| AppError::Internal("cannot encrypt the key".into()))?;
        let item = StoredIdentity {
            id: id.clone(),
            chain: chain.iter().map(|c| b64(c)).collect(),
            kdf,
            nonce: b64(&nonce),
            key: b64(&sealed),
        };
        file.v = 1;
        file.items.push(item);
        self.write(&file)?;
        describe(&chain, id, "file", true)
    }

    /// Decrypt one key for signing.
    pub fn signer(&self, id: &str, passphrase: &str) -> AppResult<SoftwareSigner> {
        use chacha20poly1305::aead::Aead;
        let file = self.read()?;
        let item = file.items.iter().find(|i| i.id == id).ok_or_else(|| {
            AppError::Unsupported("that signing identity is no longer stored".into())
        })?;
        let key = derive(passphrase, &item.kdf)?;
        let nonce: [u8; 12] = unb64(&item.nonce)?
            .try_into()
            .map_err(|_| AppError::Malformed("identity store: bad nonce".into()))?;
        let pkcs8 = cipher(&key)
            .decrypt((&nonce).into(), unb64(&item.key)?.as_slice())
            .map_err(|_| AppError::Unsupported("the passphrase is not right".into()))?;
        SoftwareSigner::from_pkcs8(&pkcs8, Self::chain_of(item)?)
    }

    pub fn remove(&self, id: &str) -> AppResult<()> {
        let mut file = self.read()?;
        let before = file.items.len();
        file.items.retain(|i| i.id != id);
        if file.items.len() == before {
            return Ok(());
        }
        self.write(&file)
    }
}
