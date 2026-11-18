//! RFC 3161 timestamps: a time-stamping server signs the hash of the
//! signature value together with the time, and the token goes into the
//! CMS as an unsigned attribute.
//!
//! This is the only place certificate signing goes online, and only when
//! the user turned on the timestamp in the signing dialog and gave a server.

use super::cms::OID_SHA256;
use crate::errors::{AppError, AppResult};
use cms::content_info::ContentInfo;
use cms::signed_data::SignedData;
use der::asn1::{Any, Int, ObjectIdentifier, OctetString};
use der::{Decode, Encode, Sequence, Tagged};
use sha2_11::{Digest, Sha256};
use spki::AlgorithmIdentifierOwned;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(20);
/// A token is a few kilobytes; anything far bigger is not one.
const MAX_RESPONSE: usize = 256 * 1024;

#[derive(Clone, Debug, Sequence)]
struct MessageImprint {
    hash_algorithm: AlgorithmIdentifierOwned,
    hashed_message: OctetString,
}

#[derive(Clone, Debug, Sequence)]
struct TimeStampReq {
    version: u8,
    message_imprint: MessageImprint,
    #[asn1(optional = "true")]
    req_policy: Option<ObjectIdentifier>,
    #[asn1(optional = "true")]
    nonce: Option<Int>,
    #[asn1(default = "Default::default")]
    cert_req: bool,
}

#[derive(Clone, Debug, Sequence)]
struct PkiStatusInfo {
    status: u8,
    #[asn1(optional = "true")]
    status_string: Option<Vec<String>>,
    #[asn1(optional = "true")]
    fail_info: Option<der::asn1::BitString>,
}

#[derive(Clone, Debug, Sequence)]
struct TimeStampResp {
    status: PkiStatusInfo,
    #[asn1(optional = "true")]
    time_stamp_token: Option<Any>,
}

fn der_err(e: der::Error) -> AppError {
    AppError::Malformed(format!("timestamp: {e}"))
}

/// A DER TimeStampReq for SHA-256 of `signature`, asking for the server's
/// certificate in the token.
pub fn request(signature: &[u8], nonce: u64) -> AppResult<Vec<u8>> {
    let mut nonce_bytes = nonce.to_be_bytes().to_vec();
    // A positive INTEGER: a leading zero when the top bit is set.
    if nonce_bytes[0] & 0x80 != 0 {
        nonce_bytes.insert(0, 0);
    }
    TimeStampReq {
        version: 1,
        message_imprint: MessageImprint {
            hash_algorithm: AlgorithmIdentifierOwned {
                oid: OID_SHA256,
                parameters: None,
            },
            hashed_message: OctetString::new(Sha256::digest(signature).to_vec())
                .map_err(der_err)?,
        },
        req_policy: None,
        nonce: Some(Int::new(&nonce_bytes).map_err(der_err)?),
        cert_req: true,
    }
    .to_der()
    .map_err(der_err)
}

/// The token in a server's answer, checked against what was asked: granted,
/// over the same hash, with the same nonce.
pub fn token_from_response(resp: &[u8], signature: &[u8], nonce: u64) -> AppResult<Vec<u8>> {
    let resp = TimeStampResp::from_der(resp).map_err(der_err)?;
    // 0 granted, 1 granted with modifications.
    if resp.status.status > 1 {
        let why = resp
            .status
            .status_string
            .map(|s| s.join("; "))
            .unwrap_or_default();
        return Err(AppError::Unsupported(format!(
            "the timestamp server refused the request{}{why}",
            if why.is_empty() { "" } else { ": " }
        )));
    }
    let token = resp
        .time_stamp_token
        .ok_or_else(|| AppError::Malformed("the timestamp server sent no token".into()))?;
    let token_der = token.to_der().map_err(der_err)?;
    let info = ContentInfo::from_der(&token_der).map_err(der_err)?;
    let signed: SignedData = info.content.decode_as().map_err(der_err)?;
    let tst = signed
        .encap_content_info
        .econtent
        .ok_or_else(|| AppError::Malformed("timestamp: no TSTInfo".into()))?
        .decode_as::<OctetString>()
        .map_err(der_err)?;
    // TSTInfo: version, policy, messageImprint, serialNumber, genTime,
    // accuracy?, ordering?, nonce?, ...
    let items: Vec<Any> = Vec::<Any>::from_der(tst.as_bytes()).map_err(der_err)?;
    let imprint: MessageImprint = items
        .get(2)
        .ok_or_else(|| AppError::Malformed("timestamp: short TSTInfo".into()))?
        .decode_as()
        .map_err(der_err)?;
    if imprint.hashed_message.as_bytes() != Sha256::digest(signature).as_slice() {
        return Err(AppError::Malformed(
            "the timestamp is for something else".into(),
        ));
    }
    let token_nonce = items
        .iter()
        .skip(5)
        .find(|a| a.tag() == der::Tag::Integer)
        .and_then(|a| a.decode_as::<Int>().ok());
    let mut want = nonce.to_be_bytes().to_vec();
    if want[0] & 0x80 != 0 {
        want.insert(0, 0);
    }
    let want = Int::new(&want).map_err(der_err)?;
    if token_nonce.as_ref() != Some(&want) {
        return Err(AppError::Malformed(
            "the timestamp answers another request".into(),
        ));
    }
    Ok(token_der)
}

/// Ask `url` for a timestamp over `signature`.
pub async fn fetch(url: &str, signature: &[u8]) -> AppResult<Vec<u8>> {
    let parsed = url::Url::parse(url.trim())
        .map_err(|_| AppError::Unsupported("the timestamp server address is not a URL".into()))?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(AppError::Unsupported(
            "the timestamp server address must start with http:// or https://".into(),
        ));
    }
    let nonce = getrandom::u64().map_err(|e| AppError::Internal(format!("no randomness: {e}")))?;
    let body = request(signature, nonce)?;
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .user_agent("SpeedyF/0.1.0 (PDF signing)")
        .build()
        .map_err(|e| AppError::Internal(format!("timestamp client: {e}")))?;
    let resp = client
        .post(parsed)
        .header("Content-Type", "application/timestamp-query")
        .body(body)
        .send()
        .await
        .map_err(|e| AppError::Io(format!("cannot reach the timestamp server: {e}")))?;
    if !resp.status().is_success() {
        return Err(AppError::Io(format!(
            "the timestamp server answered {}",
            resp.status()
        )));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| AppError::Io(format!("timestamp server: {e}")))?;
    if bytes.len() > MAX_RESPONSE {
        return Err(AppError::Malformed(
            "the timestamp answer is too large".into(),
        ));
    }
    token_from_response(&bytes, signature, nonce)
}
