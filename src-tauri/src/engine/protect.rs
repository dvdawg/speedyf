//! Password protection of saved documents: a password to open, a
//! permissions password with what it allows, and keeping a source's
//! protection across a save.
//!
//! Every save path builds an unencrypted file first (`save::build_output`
//! rewrites an encrypted source without its security handler, because the
//! form pass and the structure carry reopen the output with no password).
//! Protection is applied last, to that finished file: lopdf parses it,
//! encrypts every string and stream, and writes it back in full.
//!
//! New protection is always AES-256 (security handler revision 6, ISO
//! 32000-2). lopdf 0.47 writes revision 6 itself - /O /U /OE /UE /Perms by
//! Algorithms 8 to 10, AESV3 crypt filters, /EncryptMetadata - so all this
//! module adds is the random file key and the choice of handler.
//!
//! Keeping a source's protection (`ProtectRequest::Keep`):
//! - AES-256 sources reuse their own file key and their own /O /U /OE /UE
//!   /Perms, so the same passwords open the output with the same
//!   permissions, without anyone having to know the other password.
//! - Older handlers (RC4, AES-128) are upgraded to AES-256 when the
//!   permissions password is known, because Algorithm 7 then recovers the
//!   open password too. Opened with only the open password, the permissions
//!   password cannot be recovered, so the source's own handler is reused
//!   instead, with its /ID, which those handlers mix into the key.

use crate::errors::{AppError, AppResult};
use lopdf::encryption::crypt_filters::{Aes256CryptFilter, CryptFilter};
use lopdf::encryption::PasswordAlgorithm;
use lopdf::{
    Dictionary, Document, EncryptionState, EncryptionVersion, LoadOptions, Object, Permissions,
    StringFormat,
};
use pdfium_render::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

/// How a protected document may be printed.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum PrintLevel {
    None,
    Low,
    #[default]
    High,
}

/// What a protected document allows to someone who opened it without the
/// permissions password. The bits of /P, named.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PermissionSet {
    pub print: PrintLevel,
    pub copy: bool,
    /// Change the content: everything not covered by the three below.
    pub edit: bool,
    /// Add and change comments, and fill forms.
    pub annotate: bool,
    pub fill_forms: bool,
    /// Insert, delete and rotate pages.
    pub assemble: bool,
    /// Text for screen readers and other assistive technology.
    pub accessibility: bool,
}

impl Default for PermissionSet {
    fn default() -> Self {
        Self::all()
    }
}

impl PermissionSet {
    pub fn all() -> Self {
        PermissionSet {
            print: PrintLevel::High,
            copy: true,
            edit: true,
            annotate: true,
            fill_forms: true,
            assemble: true,
            accessibility: true,
        }
    }

    /// Read a /P value.
    pub fn from_bits(p: u32) -> Self {
        let bit = |n: u32| p & (1 << (n - 1)) != 0;
        PermissionSet {
            print: match (bit(3), bit(12)) {
                (false, _) => PrintLevel::None,
                (true, false) => PrintLevel::Low,
                (true, true) => PrintLevel::High,
            },
            edit: bit(4),
            copy: bit(5),
            annotate: bit(6),
            fill_forms: bit(9),
            accessibility: bit(10),
            assemble: bit(11),
        }
    }

    fn to_permissions(self) -> Permissions {
        let mut p = Permissions::empty();
        let mut set = |flag: Permissions, on: bool| p.set(flag, on);
        set(Permissions::PRINTABLE, self.print != PrintLevel::None);
        set(
            Permissions::PRINTABLE_IN_HIGH_QUALITY,
            self.print == PrintLevel::High,
        );
        set(Permissions::MODIFIABLE, self.edit);
        set(Permissions::COPYABLE, self.copy);
        set(Permissions::ANNOTABLE, self.annotate);
        set(Permissions::FILLABLE, self.fill_forms);
        set(Permissions::COPYABLE_FOR_ACCESSIBILITY, self.accessibility);
        set(Permissions::ASSEMBLABLE, self.assemble);
        p
    }
}

/// What a save does about protection.
#[derive(Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ProtectRequest {
    /// Whatever the source had, the output has.
    #[default]
    Keep,
    /// No encryption at all.
    Remove,
    /// New AES-256 protection. An empty password means none of that kind.
    #[serde(rename_all = "camelCase")]
    Set {
        open_password: String,
        permissions_password: String,
        permissions: PermissionSet,
    },
}

impl ProtectRequest {
    /// The password the saved file is best reopened with: the permissions
    /// password when there is one, so whoever set the protection keeps every
    /// right in this session.
    pub fn reopen_password(&self, current: Option<&str>) -> Option<String> {
        match self {
            ProtectRequest::Keep => current.map(str::to_owned),
            ProtectRequest::Remove => None,
            ProtectRequest::Set {
                open_password,
                permissions_password,
                ..
            } => [permissions_password, open_password]
                .into_iter()
                .find(|p| !p.is_empty())
                .cloned(),
        }
    }
}

/// A document's protection, as the reader shows it.
#[derive(Serialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProtectionDto {
    pub encrypted: bool,
    /// Security handler revision; 0 when not encrypted.
    pub revision: i32,
    /// "AES-256", "AES-128 or RC4 128-bit", ...; empty when not encrypted.
    pub method: String,
    /// Opening it asks for a password.
    pub open_password: bool,
    /// Nothing is withheld from this session: it was opened with the
    /// permissions (owner) password, or the file grants everything anyway.
    pub owner_unlocked: bool,
    /// What the file allows without the permissions password.
    pub permissions: PermissionSet,
    /// Digital signatures in the file, which any rewrite invalidates.
    pub signatures: u32,
}

impl ProtectionDto {
    /// Some permission is withheld from this session.
    pub fn restricted(&self) -> bool {
        self.encrypted && !self.owner_unlocked && self.permissions != PermissionSet::all()
    }
}

fn method_name(revision: i32) -> &'static str {
    match revision {
        2 => "RC4 40-bit",
        3 => "RC4 128-bit",
        4 => "128-bit (AES or RC4)",
        5 | 6 => "AES-256",
        _ => "unknown",
    }
}

/// The /P bits that grant something: print, modify, copy, annotate, fill,
/// accessibility, assemble, print in high quality.
const GRANTING_BITS: u32 = 0xF3C;

/// Whether PDFium's effective permissions withhold nothing. Once the owner
/// password unlocked a file it reports every bit set (bar the two reserved
/// low ones); a file whose /P grants everything reads the same either way,
/// and for it the difference does not matter.
fn unrestricted(effective: u32) -> bool {
    effective & GRANTING_BITS == GRANTING_BITS
}

/// Read the protection of the document open behind `raw`.
///
/// `open_password` says whether opening it took a password at all; the
/// caller knows, since it either supplied one or did not (see
/// `needs_open_password`).
pub fn read(
    b: &dyn PdfiumLibraryBindings,
    raw: FPDF_DOCUMENT,
    open_password: bool,
) -> ProtectionDto {
    let revision = b.FPDF_GetSecurityHandlerRevision(raw);
    let signatures = signature_count(b, raw);
    if revision < 0 {
        return ProtectionDto {
            permissions: PermissionSet::all(),
            signatures,
            ..Default::default()
        };
    }
    // c_ulong is 32 bits on Windows and 64 elsewhere.
    #[allow(clippy::unnecessary_cast)]
    let effective = b.FPDF_GetDocPermissions(raw) as u32;
    #[allow(clippy::unnecessary_cast)]
    let user = b.FPDF_GetDocUserPermissions(raw) as u32;
    ProtectionDto {
        encrypted: true,
        revision,
        method: method_name(revision).into(),
        open_password,
        owner_unlocked: unrestricted(effective),
        permissions: PermissionSet::from_bits(user),
        signatures,
    }
}

/// Digital signatures in the document.
///
/// The signing work going on beside this adds its own `has_signatures` in
/// engine/signing; this is the same question asked of PDFium directly,
/// kept here so protection does not depend on that module landing first.
fn signature_count(b: &dyn PdfiumLibraryBindings, raw: FPDF_DOCUMENT) -> u32 {
    b.FPDF_GetSignatureCount(raw).max(0) as u32
}

/// Whether the file at `path` asks for a password to open. Only meaningful
/// for a file that opened with `password`: without one, it plainly did not.
pub fn needs_open_password(
    b: &dyn PdfiumLibraryBindings,
    path: &str,
    password: Option<&str>,
) -> bool {
    if password.is_none_or(str::is_empty) {
        return false;
    }
    let probe = b.FPDF_LoadDocument(path, None);
    if probe.is_null() {
        return true;
    }
    b.FPDF_CloseDocument(probe);
    false
}

/// Whether `password` lifts every restriction of the file at `path`: it is
/// the permissions password, or the file restricts nothing.
pub fn is_owner_password(b: &dyn PdfiumLibraryBindings, path: &str, password: &str) -> bool {
    let probe = b.FPDF_LoadDocument(path, Some(password));
    if probe.is_null() {
        return false;
    }
    #[allow(clippy::unnecessary_cast)]
    let effective = b.FPDF_GetDocPermissions(probe) as u32;
    b.FPDF_CloseDocument(probe);
    unrestricted(effective)
}

/// How a save's output is to be encrypted.
#[derive(Clone, Debug, Default)]
pub struct Target {
    state: Option<EncryptionState>,
    /// The /ID the output must carry: older handlers mix its first element
    /// into the key and /U, so reusing one means reusing its /ID.
    id: Option<Object>,
}

impl Target {
    pub fn encrypts(&self) -> bool {
        self.state.is_some()
    }

    /// The security handler revision the output will report.
    pub fn revision(&self) -> Option<i64> {
        self.state.as_ref().map(EncryptionState::revision)
    }
}

/// Decide how the output of a save is encrypted.
///
/// `source_encrypted` is what PDFium said of the open source; an
/// unencrypted source needs nothing read for `Keep`.
pub fn plan(
    request: &ProtectRequest,
    src_path: &Path,
    password: Option<&str>,
    source_encrypted: bool,
) -> AppResult<Target> {
    match request {
        ProtectRequest::Remove => Ok(Target::default()),
        ProtectRequest::Keep if !source_encrypted => Ok(Target::default()),
        ProtectRequest::Keep => keep(src_path, password.unwrap_or("")),
        ProtectRequest::Set {
            open_password,
            permissions_password,
            permissions,
        } => {
            if open_password.is_empty() && permissions_password.is_empty() {
                return Err(AppError::Unsupported(
                    "protection needs a password to open or a permissions password".into(),
                ));
            }
            if !permissions_password.is_empty() && permissions_password == open_password {
                return Err(AppError::Unsupported(
                    "the permissions password must differ from the password to open".into(),
                ));
            }
            // Without a permissions password, the open password is the
            // owner's too, and whoever opens the file may do anything:
            // permissions nobody could ever lift are not offered.
            let (owner, permissions) = if permissions_password.is_empty() {
                (open_password.as_str(), PermissionSet::all())
            } else {
                (permissions_password.as_str(), *permissions)
            };
            Ok(Target {
                state: Some(aes256(open_password, owner, permissions.to_permissions())?),
                id: None,
            })
        }
    }
}

/// A fresh AES-256 (revision 6) handler with a random file key.
fn aes256(user: &str, owner: &str, permissions: Permissions) -> AppResult<EncryptionState> {
    let mut key = [0u8; 32];
    getrandom::fill(&mut key)
        .map_err(|e| AppError::Internal(format!("no randomness for a file key: {e}")))?;
    let filter: Arc<dyn CryptFilter> = Arc::new(Aes256CryptFilter);
    let crypt_filters = BTreeMap::from([(b"StdCF".to_vec(), filter)]);
    let version = EncryptionVersion::V5 {
        encrypt_metadata: true,
        crypt_filters,
        file_encryption_key: &key,
        stream_filter: b"StdCF".to_vec(),
        string_filter: b"StdCF".to_vec(),
        owner_password: owner,
        user_password: user,
        permissions,
    };
    EncryptionState::try_from(version).map_err(|e| AppError::Internal(format!("encryption: {e}")))
}

fn keep(src_path: &Path, password: &str) -> AppResult<Target> {
    let src = Document::load_with_options(src_path, LoadOptions::with_password(password)).map_err(
        |e| {
            AppError::Malformed(format!(
                "cannot read the source's protection ({e}); Protect... can remove or replace it"
            ))
        },
    )?;
    let Some(state) = src.encryption_state.clone() else {
        return Err(AppError::Unsupported(
            "the source's protection could not be read".into(),
        ));
    };
    let id = src.trailer.get(b"ID").ok().cloned();
    if state.revision() >= 5 {
        return Ok(Target {
            state: Some(state),
            id: None,
        });
    }
    // Revision 4 and older: with the permissions password in hand, recover
    // the open password and move up to AES-256.
    if let Some(open) = recovered_open_password(&state, id.as_ref(), password) {
        let permissions = Permissions::from_bits_truncate(state.permissions().bits());
        return Ok(Target {
            state: Some(aes256(&open, password, permissions)?),
            id: None,
        });
    }
    Ok(Target {
        state: Some(state),
        id,
    })
}

/// The open password of an older (revision 2 to 4) handler, when
/// `password` is its permissions password; `None` otherwise.
fn recovered_open_password(
    state: &EncryptionState,
    id: Option<&Object>,
    password: &str,
) -> Option<String> {
    // The algorithms read the handler from a document's trailer; a stub
    // holding just the /Encrypt dictionary and the /ID is enough.
    let mut stub = Document::with_version("1.7");
    let encrypt = state.encode().ok()?;
    let encrypt_id = stub.add_object(Object::Dictionary(encrypt));
    stub.trailer.set("Encrypt", Object::Reference(encrypt_id));
    if let Some(id) = id {
        stub.trailer.set("ID", id.clone());
    }
    let algorithm = PasswordAlgorithm::try_from(&stub).ok()?;
    let sanitized = algorithm.sanitize_password(password).ok()?;
    algorithm
        .authenticate_owner_password(&stub, &sanitized)
        .ok()?;
    let padded = algorithm
        .resolve_password_for_key_derivation(&stub, &sanitized)
        .ok()?;
    // Revision 4 and older passwords are bytes in PDFDocEncoding, which
    // agrees with Latin-1 on everything a keyboard types.
    Some(strip_padding(&padded).iter().map(|&b| b as char).collect())
}

/// The padding Algorithm 2 appends to a password (ISO 32000-1, 7.6.3.3).
const PAD: [u8; 32] = [
    0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01, 0x08,
    0x2E, 0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80, 0x2F, 0x0C, 0xA9, 0xFE, 0x64, 0x53, 0x69, 0x7A,
];

/// A recovered user password without the padding it was stored with.
fn strip_padding(padded: &[u8]) -> &[u8] {
    if padded.len() != PAD.len() {
        return padded;
    }
    (0..=PAD.len())
        .find(|&n| padded[n..] == PAD[..PAD.len() - n])
        .map_or(padded, |n| &padded[..n])
}

/// Encrypt the finished, unencrypted file at `path` in place.
pub fn encrypt_file(path: &Path, target: &Target) -> AppResult<()> {
    let Some(state) = target.state.as_ref() else {
        return Ok(());
    };
    let mut doc = Document::load(path)
        .map_err(|e| AppError::Internal(format!("protect: cannot read the output: {e}")))?;
    if doc.is_encrypted() {
        return Err(AppError::Internal(
            "protect: the output is already encrypted".into(),
        ));
    }
    // Their members were unpacked on load, and the writer emits every object
    // on its own; encrypting the containers would only waste the work.
    doc.objects.retain(|_, object| {
        !matches!(
            object.type_name(),
            Ok(b"ObjStm") | Ok(b"XRef") | Ok(b"Linearized")
        )
    });
    match &target.id {
        Some(id) => doc.trailer.set("ID", id.clone()),
        None if !doc.trailer.has(b"ID") => doc.trailer.set("ID", fresh_id()?),
        None => {}
    }
    if state.revision() >= 5 {
        declare_aes256(&mut doc);
    }
    doc.encrypt(state)
        .map_err(|e| AppError::Internal(format!("protect: encryption failed: {e}")))?;
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes)
        .map_err(|e| AppError::Internal(format!("protect: write failed: {e}")))?;
    let mut file = std::fs::File::create(path)?;
    std::io::Write::write_all(&mut file, &bytes)?;
    file.sync_all()?;
    Ok(())
}

/// AES-256 arrived in PDF 2.0, and before that as Adobe's extension level 8
/// to PDF 1.7; older readers look for one or the other before trying it.
fn declare_aes256(doc: &mut Document) {
    if doc.version.as_str() >= "2.0" {
        return;
    }
    if doc.version.as_str() < "1.7" {
        doc.version = "1.7".into();
    }
    if let Ok(catalog) = doc.catalog_mut() {
        let mut adbe = Dictionary::new();
        adbe.set("BaseVersion", Object::Name(b"1.7".to_vec()));
        adbe.set("ExtensionLevel", Object::Integer(8));
        let mut extensions = match catalog.get(b"Extensions") {
            Ok(Object::Dictionary(existing)) => existing.clone(),
            _ => Dictionary::new(),
        };
        extensions.set("ADBE", Object::Dictionary(adbe));
        catalog.set("Extensions", Object::Dictionary(extensions));
    }
}

fn fresh_id() -> AppResult<Object> {
    let mut id = [0u8; 16];
    getrandom::fill(&mut id)
        .map_err(|e| AppError::Internal(format!("no randomness for a file id: {e}")))?;
    let part = Object::String(id.to_vec(), StringFormat::Hexadecimal);
    Ok(Object::Array(vec![part.clone(), part]))
}

#[cfg(test)]
pub(crate) mod tests;
