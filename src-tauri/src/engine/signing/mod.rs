//! Digital signatures: finding them (`parse`), checking them (`verify`) and
//! telling what changed after each one was made (`changes`).
//!
//! Everything here is offline. Trust in a signer's certificate comes from the
//! system's own roots on macOS; elsewhere a signature is checked for
//! integrity and the signer named, with the identity reported as not checked.

pub mod parse;
pub mod verify;

use std::path::Path;

/// Whether the PDF in `bytes` carries at least one signature value.
///
/// A document with signatures must never be fully rewritten: a rewrite
/// renumbers and re-serialises every object, so no signed byte range survives
/// it. Anything that would rewrite a file (a restructuring save, encryption,
/// redaction) should ask this first.
pub fn has_signatures(bytes: &[u8]) -> bool {
    parse::has_signature_bytes(bytes) && !parse::signatures_in(bytes).is_empty()
}

/// `has_signatures` for a file on disk. A file that cannot be read has none.
pub fn file_has_signatures(path: &Path) -> bool {
    std::fs::read(path).is_ok_and(|bytes| has_signatures(&bytes))
}
