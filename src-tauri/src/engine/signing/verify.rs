//! Checking a signature's cryptography: the CMS `SignedData` in `/Contents`,
//! the signer's certificate, an embedded RFC 3161 timestamp, and - on macOS -
//! whether the certificate chains to a root the system trusts.
//!
//! What is checked, for each signature:
//! - the digest of the signed byte range equals the `messageDigest` the
//!   signer signed (or, without signed attributes, the digest signed directly);
//! - the signature over the signed attributes verifies with the public key of
//!   the certificate the signer names;
//! - a timestamp token's message imprint equals the digest of the signature
//!   it stamps, and the token's own signature verifies;
//! - on macOS only, the certificate chain is evaluated against the system's
//!   trusted roots at the signing time, with network fetching turned off.
//!
//! Nothing here goes online: no OCSP, no CRL, no AIA certificate fetching.
//!
//! The DER walking is done by hand on purpose. The `cms` types sort a SET OF
//! on decode, which is right for DER but would change the bytes a signer
//! actually signed when it did not sort its attributes; the raw slices are
//! what get hashed.

use super::parse::SignatureInfo;
use cms::signed_data::{SignerIdentifier, SignerInfo};
use der::asn1::ObjectIdentifier as Oid;
use der::{Decode, Encode, Tagged};
use sha1::Sha1;
use sha2::{Digest, Sha224, Sha256, Sha384, Sha512};
use x509_cert::Certificate;

const OID_SIGNED_DATA: Oid = Oid::new_unwrap("1.2.840.113549.1.7.2");
const OID_CONTENT_TYPE: Oid = Oid::new_unwrap("1.2.840.113549.1.9.3");
const OID_MESSAGE_DIGEST: Oid = Oid::new_unwrap("1.2.840.113549.1.9.4");
const OID_SIGNING_TIME: Oid = Oid::new_unwrap("1.2.840.113549.1.9.5");
const OID_TIMESTAMP_TOKEN: Oid = Oid::new_unwrap("1.2.840.113549.1.9.16.2.14");
const OID_TST_INFO: Oid = Oid::new_unwrap("1.2.840.113549.1.9.16.1.4");

const OID_SHA1: Oid = Oid::new_unwrap("1.3.14.3.2.26");
const OID_SHA224: Oid = Oid::new_unwrap("2.16.840.1.101.3.4.2.4");
const OID_SHA256: Oid = Oid::new_unwrap("2.16.840.1.101.3.4.2.1");
const OID_SHA384: Oid = Oid::new_unwrap("2.16.840.1.101.3.4.2.2");
const OID_SHA512: Oid = Oid::new_unwrap("2.16.840.1.101.3.4.2.3");

const OID_RSA: Oid = Oid::new_unwrap("1.2.840.113549.1.1.1");
const OID_RSA_SHA1: Oid = Oid::new_unwrap("1.2.840.113549.1.1.5");
const OID_RSA_SHA256: Oid = Oid::new_unwrap("1.2.840.113549.1.1.11");
const OID_RSA_SHA384: Oid = Oid::new_unwrap("1.2.840.113549.1.1.12");
const OID_RSA_SHA512: Oid = Oid::new_unwrap("1.2.840.113549.1.1.13");
const OID_RSA_SHA224: Oid = Oid::new_unwrap("1.2.840.113549.1.1.14");
const OID_RSA_PSS: Oid = Oid::new_unwrap("1.2.840.113549.1.1.10");
const OID_EC_PUBLIC_KEY: Oid = Oid::new_unwrap("1.2.840.10045.2.1");
const OID_ECDSA_SHA1: Oid = Oid::new_unwrap("1.2.840.10045.4.1");
const OID_ECDSA_SHA224: Oid = Oid::new_unwrap("1.2.840.10045.4.3.1");
const OID_ECDSA_SHA256: Oid = Oid::new_unwrap("1.2.840.10045.4.3.2");
const OID_ECDSA_SHA384: Oid = Oid::new_unwrap("1.2.840.10045.4.3.3");
const OID_ECDSA_SHA512: Oid = Oid::new_unwrap("1.2.840.10045.4.3.4");
const OID_P256: Oid = Oid::new_unwrap("1.2.840.10045.3.1.7");
const OID_P384: Oid = Oid::new_unwrap("1.3.132.0.34");

const OID_CN: Oid = Oid::new_unwrap("2.5.4.3");
const OID_O: Oid = Oid::new_unwrap("2.5.4.10");
const OID_EMAIL: Oid = Oid::new_unwrap("1.2.840.113549.1.9.1");
const OID_SAN: Oid = Oid::new_unwrap("2.5.29.17");
const OID_SKI: Oid = Oid::new_unwrap("2.5.29.14");

/// The digest algorithms PDF signers use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DigestAlg {
    Sha1,
    Sha224,
    Sha256,
    Sha384,
    Sha512,
}

impl DigestAlg {
    pub fn from_oid(oid: &Oid) -> Option<Self> {
        Some(match *oid {
            OID_SHA1 => DigestAlg::Sha1,
            OID_SHA224 => DigestAlg::Sha224,
            OID_SHA256 => DigestAlg::Sha256,
            OID_SHA384 => DigestAlg::Sha384,
            OID_SHA512 => DigestAlg::Sha512,
            _ => return None,
        })
    }

    pub fn oid(self) -> Oid {
        match self {
            DigestAlg::Sha1 => OID_SHA1,
            DigestAlg::Sha224 => OID_SHA224,
            DigestAlg::Sha256 => OID_SHA256,
            DigestAlg::Sha384 => OID_SHA384,
            DigestAlg::Sha512 => OID_SHA512,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            DigestAlg::Sha1 => "SHA-1",
            DigestAlg::Sha224 => "SHA-224",
            DigestAlg::Sha256 => "SHA-256",
            DigestAlg::Sha384 => "SHA-384",
            DigestAlg::Sha512 => "SHA-512",
        }
    }

    /// Digest the concatenation of `parts`.
    pub fn digest(self, parts: &[&[u8]]) -> Vec<u8> {
        fn run<D: Digest>(parts: &[&[u8]]) -> Vec<u8> {
            let mut hasher = D::new();
            for part in parts {
                hasher.update(part);
            }
            hasher.finalize().to_vec()
        }
        match self {
            DigestAlg::Sha1 => run::<Sha1>(parts),
            DigestAlg::Sha224 => run::<Sha224>(parts),
            DigestAlg::Sha256 => run::<Sha256>(parts),
            DigestAlg::Sha384 => run::<Sha384>(parts),
            DigestAlg::Sha512 => run::<Sha512>(parts),
        }
    }
}

// ---------------------------------------------------------------------------
// Minimal DER walking

/// One DER element: its tag byte, the whole encoding and the value inside.
#[derive(Clone, Copy)]
struct Tlv<'a> {
    tag: u8,
    whole: &'a [u8],
    value: &'a [u8],
}

/// Read the element at the start of `bytes`. Definite lengths only.
fn tlv(bytes: &[u8]) -> Option<Tlv<'_>> {
    let tag = *bytes.first()?;
    if tag & 0x1f == 0x1f {
        return None; // multi-byte tags never occur in the structures read here
    }
    let first = *bytes.get(1)?;
    let (len, head) = if first < 0x80 {
        (first as usize, 2)
    } else {
        let n = (first & 0x7f) as usize;
        if n == 0 || n > 4 {
            return None;
        }
        let mut len = 0usize;
        for i in 0..n {
            len = len << 8 | *bytes.get(2 + i)? as usize;
        }
        (len, 2 + n)
    };
    let whole = bytes.get(..head.checked_add(len)?)?;
    Some(Tlv {
        tag,
        whole,
        value: &whole[head..],
    })
}

/// The elements inside a constructed value, in order.
fn children(value: &[u8]) -> Option<Vec<Tlv<'_>>> {
    let mut out = Vec::new();
    let mut rest = value;
    while !rest.is_empty() {
        let next = tlv(rest)?;
        rest = &rest[next.whole.len()..];
        out.push(next);
    }
    Some(out)
}

/// The DER blob at the start of a signature's `/Contents`, without the zero
/// padding a signer reserves after it.
pub fn trim_der(contents: &[u8]) -> Option<&[u8]> {
    tlv(contents).map(|t| t.whole)
}

/// The raw pieces of a CMS `SignedData`.
struct RawSignedData<'a> {
    econtent_type: Option<Oid>,
    econtent: Option<Vec<u8>>,
    certificates: Vec<&'a [u8]>,
    signer_infos: Vec<&'a [u8]>,
}

fn raw_signed_data(der: &[u8]) -> Result<RawSignedData<'_>, String> {
    let bad = || "the signature value is not a CMS SignedData structure".to_string();
    let info = tlv(der).filter(|t| t.tag == 0x30).ok_or_else(bad)?;
    let parts = children(info.value).ok_or_else(bad)?;
    let oid = parts.first().filter(|t| t.tag == 0x06).ok_or_else(bad)?;
    if Oid::from_der(oid.whole).ok() != Some(OID_SIGNED_DATA) {
        return Err(bad());
    }
    let explicit = parts.get(1).filter(|t| t.tag == 0xa0).ok_or_else(bad)?;
    let signed = tlv(explicit.value).filter(|t| t.tag == 0x30).ok_or_else(bad)?;
    let fields = children(signed.value).ok_or_else(bad)?;
    // version, digestAlgorithms, encapContentInfo, [0] certs?, [1] crls?, signerInfos
    let encap = fields.get(2).filter(|t| t.tag == 0x30).ok_or_else(bad)?;
    let encap_parts = children(encap.value).ok_or_else(bad)?;
    let econtent_type = encap_parts.first().and_then(|t| Oid::from_der(t.whole).ok());
    let econtent = encap_parts
        .get(1)
        .filter(|t| t.tag == 0xa0)
        .and_then(|t| tlv(t.value))
        .and_then(|octets| match octets.tag {
            0x04 => Some(octets.value.to_vec()),
            // A constructed OCTET STRING (BER) in pieces.
            0x24 => children(octets.value).map(|pieces| {
                pieces
                    .iter()
                    .flat_map(|p| p.value.iter().copied())
                    .collect::<Vec<u8>>()
            }),
            _ => None,
        });
    let mut certificates = Vec::new();
    let mut signer_infos = Vec::new();
    for field in &fields[3..] {
        match field.tag {
            0xa0 => {
                for cert in children(field.value).ok_or_else(bad)? {
                    if cert.tag == 0x30 {
                        certificates.push(cert.whole);
                    }
                }
            }
            0x31 => {
                for info in children(field.value).ok_or_else(bad)? {
                    signer_infos.push(info.whole);
                }
            }
            _ => {}
        }
    }
    if signer_infos.is_empty() {
        return Err("the signature value names no signer".into());
    }
    Ok(RawSignedData {
        econtent_type,
        econtent,
        certificates,
        signer_infos,
    })
}

/// The DER of a SignerInfo's signed attributes as they are hashed: the
/// `[0] IMPLICIT` tag replaced by the SET OF tag, bytes otherwise untouched.
fn raw_signed_attrs(signer_info: &[u8]) -> Option<Vec<u8>> {
    let info = tlv(signer_info)?;
    let attrs = children(info.value)?
        .into_iter()
        .find(|t| t.tag == 0xa0)?;
    let mut out = attrs.whole.to_vec();
    out[0] = 0x31;
    Some(out)
}

// ---------------------------------------------------------------------------
// Signature algorithms

/// Verify `signature` over `message_digest` (already hashed with `digest`)
/// with the key in `cert`, using the algorithm `sig_alg` names.
fn verify_with_cert(
    cert: &Certificate,
    sig_alg: &Oid,
    digest: DigestAlg,
    message_digest: &[u8],
    signature: &[u8],
) -> Result<(), String> {
    let spki = &cert.tbs_certificate.subject_public_key_info;
    let key_bytes = spki.subject_public_key.raw_bytes();
    let rsa = [
        OID_RSA,
        OID_RSA_SHA1,
        OID_RSA_SHA224,
        OID_RSA_SHA256,
        OID_RSA_SHA384,
        OID_RSA_SHA512,
    ];
    let ecdsa = [
        OID_EC_PUBLIC_KEY,
        OID_ECDSA_SHA1,
        OID_ECDSA_SHA224,
        OID_ECDSA_SHA256,
        OID_ECDSA_SHA384,
        OID_ECDSA_SHA512,
    ];
    if rsa.contains(sig_alg) {
        use rsa::pkcs1::DecodeRsaPublicKey;
        use rsa::{Pkcs1v15Sign, RsaPublicKey};
        if spki.algorithm.oid != OID_RSA {
            return Err("the certificate does not hold an RSA key".into());
        }
        let key = RsaPublicKey::from_pkcs1_der(key_bytes)
            .map_err(|_| "the certificate's RSA key cannot be read".to_string())?;
        let scheme = match digest {
            DigestAlg::Sha1 => Pkcs1v15Sign::new::<Sha1>(),
            DigestAlg::Sha224 => Pkcs1v15Sign::new::<Sha224>(),
            DigestAlg::Sha256 => Pkcs1v15Sign::new::<Sha256>(),
            DigestAlg::Sha384 => Pkcs1v15Sign::new::<Sha384>(),
            DigestAlg::Sha512 => Pkcs1v15Sign::new::<Sha512>(),
        };
        key.verify(scheme, message_digest, signature)
            .map_err(|_| "the signature does not match the signer's key".into())
    } else if ecdsa.contains(sig_alg) {
        use ecdsa::signature::hazmat::PrehashVerifier;
        if spki.algorithm.oid != OID_EC_PUBLIC_KEY {
            return Err("the certificate does not hold an EC key".into());
        }
        let curve = spki
            .algorithm
            .parameters
            .as_ref()
            .and_then(|p| p.decode_as::<Oid>().ok());
        let mismatch = || "the signature does not match the signer's key".to_string();
        match curve {
            Some(OID_P256) => {
                let key = p256::ecdsa::VerifyingKey::from_sec1_bytes(key_bytes)
                    .map_err(|_| "the certificate's EC key cannot be read".to_string())?;
                let sig = p256::ecdsa::Signature::from_der(signature).map_err(|_| mismatch())?;
                key.verify_prehash(message_digest, &sig).map_err(|_| mismatch())
            }
            Some(OID_P384) => {
                let key = p384::ecdsa::VerifyingKey::from_sec1_bytes(key_bytes)
                    .map_err(|_| "the certificate's EC key cannot be read".to_string())?;
                let sig = p384::ecdsa::Signature::from_der(signature).map_err(|_| mismatch())?;
                key.verify_prehash(message_digest, &sig).map_err(|_| mismatch())
            }
            _ => Err("the signer's elliptic curve is not supported".into()),
        }
    } else if *sig_alg == OID_RSA_PSS {
        Err("RSA-PSS signatures are not supported".into())
    } else {
        Err(format!("the signature algorithm {sig_alg} is not supported"))
    }
}

/// The digest a signature algorithm fixes, when it fixes one.
fn digest_of_sig_alg(oid: &Oid) -> Option<DigestAlg> {
    Some(match *oid {
        OID_RSA_SHA1 | OID_ECDSA_SHA1 => DigestAlg::Sha1,
        OID_RSA_SHA224 | OID_ECDSA_SHA224 => DigestAlg::Sha224,
        OID_RSA_SHA256 | OID_ECDSA_SHA256 => DigestAlg::Sha256,
        OID_RSA_SHA384 | OID_ECDSA_SHA384 => DigestAlg::Sha384,
        OID_RSA_SHA512 | OID_ECDSA_SHA512 => DigestAlg::Sha512,
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// SignedData

/// What checking one CMS `SignedData` found.
#[derive(Clone, Debug, Default)]
pub struct CmsCheck {
    /// The content's digest equals the signed `messageDigest`.
    pub digest_ok: bool,
    /// The signature verifies with the signer certificate's key.
    pub signature_ok: bool,
    /// The first thing that failed, in words.
    pub problem: Option<String>,
    /// The signer's certificate, then every other certificate carried.
    pub chain: Vec<Vec<u8>>,
    /// The signed `signingTime` attribute, as Unix seconds.
    pub signing_time: Option<i64>,
    /// The encapsulated content, when there is one (a timestamp's TSTInfo).
    pub econtent: Option<Vec<u8>>,
    /// The raw signature value, which a timestamp token stamps.
    pub signature_value: Vec<u8>,
    /// An RFC 3161 timestamp token in the unsigned attributes.
    pub timestamp_token: Option<Vec<u8>>,
    pub digest: Option<DigestAlg>,
}

impl CmsCheck {
    pub fn intact(&self) -> bool {
        self.digest_ok && self.signature_ok
    }

    fn fail(mut self, problem: impl Into<String>) -> Self {
        self.problem.get_or_insert_with(|| problem.into());
        self
    }
}

/// What the signed content is, for `check_signed_data`.
pub enum Content<'a> {
    /// Detached: the signed data is these byte spans, concatenated.
    Detached(&'a [&'a [u8]]),
    /// Encapsulated: the content is inside the SignedData itself.
    Encapsulated,
}

/// Check the CMS `SignedData` in `der` against `content`.
pub fn check_signed_data(der: &[u8], content: Content<'_>) -> CmsCheck {
    let mut check = CmsCheck::default();
    let raw = match raw_signed_data(der) {
        Ok(raw) => raw,
        Err(problem) => return check.fail(problem),
    };
    check.econtent = raw.econtent.clone();
    let certs: Vec<(Certificate, &[u8])> = raw
        .certificates
        .iter()
        .filter_map(|c| Certificate::from_der(c).ok().map(|cert| (cert, *c)))
        .collect();
    let raw_info = raw.signer_infos[0];
    let info = match SignerInfo::from_der(raw_info) {
        Ok(info) => info,
        Err(_) => return check.fail("the signer information cannot be read"),
    };
    check.signature_value = info.signature.as_bytes().to_vec();
    if let Some(unsigned) = &info.unsigned_attrs {
        check.timestamp_token = unsigned
            .iter()
            .find(|a| a.oid == OID_TIMESTAMP_TOKEN)
            .and_then(|a| a.values.iter().next())
            .and_then(|v| v.to_der().ok());
    }
    let Some(digest) = DigestAlg::from_oid(&info.digest_alg.oid) else {
        return check.fail(format!(
            "the digest algorithm {} is not supported",
            info.digest_alg.oid
        ));
    };
    check.digest = Some(digest);

    // The signer's certificate: the one the SignerIdentifier names.
    let signer = certs.iter().position(|(cert, _)| match &info.sid {
        SignerIdentifier::IssuerAndSerialNumber(id) => {
            cert.tbs_certificate.issuer == id.issuer
                && cert.tbs_certificate.serial_number == id.serial_number
        }
        SignerIdentifier::SubjectKeyIdentifier(ski) => subject_key_id(cert)
            .is_some_and(|own| own == ski.0.as_bytes()),
    });
    if let Some(index) = signer {
        check.chain.push(certs[index].1.to_vec());
    }
    for (i, (_, der)) in certs.iter().enumerate() {
        if Some(i) != signer {
            check.chain.push(der.to_vec());
        }
    }

    // What the signer's digest covers.
    let content_digest = match content {
        Content::Detached(spans) => digest.digest(spans),
        Content::Encapsulated => match &raw.econtent {
            Some(econtent) => digest.digest(&[econtent]),
            None => return check.fail("the signature carries no content"),
        },
    };

    // With signed attributes, the content digest is one of them and the
    // signature covers the attributes; without, it covers the digest.
    let signed_digest = match &info.signed_attrs {
        Some(attrs) => {
            let message_digest = attrs
                .iter()
                .find(|a| a.oid == OID_MESSAGE_DIGEST)
                .and_then(|a| a.values.iter().next())
                .and_then(|v| v.decode_as::<der::asn1::OctetString>().ok());
            check.digest_ok = message_digest
                .as_ref()
                .is_some_and(|d| d.as_bytes() == content_digest.as_slice());
            if message_digest.is_none() {
                return check.fail("the signer did not sign a message digest");
            }
            if !attrs.iter().any(|a| a.oid == OID_CONTENT_TYPE) {
                check.problem = Some("the signed attributes have no content type".into());
            }
            check.signing_time = attrs
                .iter()
                .find(|a| a.oid == OID_SIGNING_TIME)
                .and_then(|a| a.values.iter().next())
                .and_then(|v| x509_cert::time::Time::from_der(&v.to_der().ok()?).ok())
                .map(|t| t.to_unix_duration().as_secs() as i64);
            let Some(raw_attrs) = raw_signed_attrs(raw_info) else {
                return check.fail("the signed attributes cannot be read");
            };
            let hash = digest_of_sig_alg(&info.signature_algorithm.oid).unwrap_or(digest);
            (hash, hash.digest(&[&raw_attrs]))
        }
        None => {
            check.digest_ok = true;
            (digest, content_digest.clone())
        }
    };
    if !check.digest_ok {
        check = check.fail("the document's bytes do not match what was signed");
    }

    let candidates: Vec<&Certificate> = match signer {
        Some(index) => vec![&certs[index].0],
        None => certs.iter().map(|(c, _)| c).collect(),
    };
    if candidates.is_empty() {
        return check.fail("the signature does not include the signer's certificate");
    }
    let mut last_error = None;
    for (n, cert) in candidates.iter().enumerate() {
        match verify_with_cert(
            cert,
            &info.signature_algorithm.oid,
            signed_digest.0,
            &signed_digest.1,
            info.signature.as_bytes(),
        ) {
            Ok(()) => {
                check.signature_ok = true;
                if signer.is_none() {
                    // Put the certificate that verified first.
                    let der = cert.to_der().unwrap_or_default();
                    check.chain.retain(|c| *c != der);
                    check.chain.insert(0, der);
                }
                let _ = n;
                break;
            }
            Err(e) => last_error = Some(e),
        }
    }
    if !check.signature_ok {
        check = check.fail(last_error.unwrap_or_else(|| "the signature does not verify".into()));
    }
    let _ = raw.econtent_type;
    check
}

fn subject_key_id(cert: &Certificate) -> Option<Vec<u8>> {
    let ext = cert
        .tbs_certificate
        .extensions
        .as_ref()?
        .iter()
        .find(|e| e.extn_id == OID_SKI)?;
    der::asn1::OctetString::from_der(ext.extn_value.as_bytes())
        .ok()
        .map(|o| o.as_bytes().to_vec())
}

// ---------------------------------------------------------------------------
// Timestamps

/// What checking an RFC 3161 timestamp token found.
#[derive(Clone, Debug)]
pub struct TimestampCheck {
    /// The token's time, as Unix seconds.
    pub time: Option<i64>,
    /// The imprint matches and the token's own signature verifies.
    pub valid: bool,
    pub problem: Option<String>,
    /// The timestamp authority's certificate first.
    pub chain: Vec<Vec<u8>>,
    /// The authority's name.
    pub authority: Option<String>,
}

/// Check a timestamp token over `stamped` (the bytes whose digest the token's
/// message imprint should hold).
pub fn check_timestamp(token: &[u8], stamped: &[&[u8]]) -> TimestampCheck {
    let cms = check_signed_data(token, Content::Encapsulated);
    let mut out = TimestampCheck {
        time: None,
        valid: false,
        problem: cms.problem.clone(),
        authority: cms
            .chain
            .first()
            .and_then(|c| Certificate::from_der(c).ok())
            .and_then(|c| Identity::of(&c).name),
        chain: cms.chain.clone(),
    };
    let Some(tst) = cms.econtent.as_deref().and_then(tst_info) else {
        out.problem
            .get_or_insert_with(|| "the timestamp token cannot be read".into());
        return out;
    };
    out.time = tst.time;
    let imprint_ok = DigestAlg::from_oid(&tst.imprint_alg)
        .is_some_and(|alg| alg.digest(stamped) == tst.imprint);
    if !imprint_ok {
        out.problem = Some("the timestamp is for different data".into());
    }
    out.valid = imprint_ok && cms.intact();
    out
}

struct TstInfo {
    imprint_alg: Oid,
    imprint: Vec<u8>,
    time: Option<i64>,
}

/// The parts of a TSTInfo a check needs:
/// `SEQUENCE { version, policy, messageImprint, serialNumber, genTime, ... }`.
fn tst_info(der: &[u8]) -> Option<TstInfo> {
    let seq = tlv(der).filter(|t| t.tag == 0x30)?;
    let fields = children(seq.value)?;
    let imprint = fields.get(2).filter(|t| t.tag == 0x30)?;
    let imprint_parts = children(imprint.value)?;
    let alg = imprint_parts.first().filter(|t| t.tag == 0x30)?;
    let alg_oid = children(alg.value)?
        .first()
        .and_then(|t| Oid::from_der(t.whole).ok())?;
    let hashed = imprint_parts.get(1).filter(|t| t.tag == 0x04)?;
    let time = fields
        .get(4)
        .and_then(|t| der::asn1::GeneralizedTime::from_der(t.whole).ok())
        .map(|t| t.to_unix_duration().as_secs() as i64);
    Some(TstInfo {
        imprint_alg: alg_oid,
        imprint: hashed.value.to_vec(),
        time,
    })
}

/// Whether `der` is a timestamp token (its content is a TSTInfo).
pub fn is_timestamp_token(der: &[u8]) -> bool {
    raw_signed_data(der).is_ok_and(|raw| raw.econtent_type == Some(OID_TST_INFO))
}

// ---------------------------------------------------------------------------
// Identity

/// Who a certificate says its subject is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Identity {
    pub name: Option<String>,
    pub email: Option<String>,
    pub organization: Option<String>,
    pub issuer: Option<String>,
    /// Validity, as Unix seconds.
    pub not_before: Option<i64>,
    pub not_after: Option<i64>,
    pub self_signed: bool,
}

impl Identity {
    pub fn of(cert: &Certificate) -> Identity {
        let tbs = &cert.tbs_certificate;
        let subject = |oid: Oid| name_attr(&tbs.subject, oid);
        let email = subject(OID_EMAIL).or_else(|| {
            use x509_cert::ext::pkix::name::GeneralName;
            use x509_cert::ext::pkix::SubjectAltName;
            let ext = tbs.extensions.as_ref()?.iter().find(|e| e.extn_id == OID_SAN)?;
            let san = SubjectAltName::from_der(ext.extn_value.as_bytes()).ok()?;
            san.0.iter().find_map(|name| match name {
                GeneralName::Rfc822Name(mail) => Some(mail.to_string()),
                _ => None,
            })
        });
        Identity {
            name: subject(OID_CN),
            email,
            organization: subject(OID_O),
            issuer: name_attr(&tbs.issuer, OID_CN).or_else(|| name_attr(&tbs.issuer, OID_O)),
            not_before: Some(tbs.validity.not_before.to_unix_duration().as_secs() as i64),
            not_after: Some(tbs.validity.not_after.to_unix_duration().as_secs() as i64),
            self_signed: tbs.issuer == tbs.subject,
        }
    }
}

fn name_attr(name: &x509_cert::name::Name, oid: Oid) -> Option<String> {
    name.0
        .iter()
        .flat_map(|rdn| rdn.0.iter())
        .find(|atv| atv.oid == oid)
        .and_then(|atv| any_string(&atv.value))
}

/// The text of a directory string, whichever string type it was written as.
fn any_string(any: &der::Any) -> Option<String> {
    use der::Tag;
    let bytes = any.value();
    let text = match any.tag() {
        Tag::Utf8String | Tag::PrintableString | Tag::Ia5String | Tag::VisibleString => {
            String::from_utf8_lossy(bytes).into_owned()
        }
        // T.61 is close enough to Latin-1 for names.
        Tag::TeletexString => bytes.iter().map(|&b| b as char).collect(),
        Tag::BmpString => {
            let units: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect();
            String::from_utf16_lossy(&units)
        }
        _ => return None,
    };
    let text = text.trim().to_string();
    (!text.is_empty()).then_some(text)
}

// ---------------------------------------------------------------------------
// Trust

/// Whether the signer's certificate chains to a root the system trusts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Trust {
    Trusted,
    /// The system was asked and said no, for this reason.
    NotTrusted(String),
    /// This platform has no trust store SpeedyF asks.
    NotChecked,
}

/// Evaluate `chain` (signer first) against the system's trusted roots at
/// `at` (Unix seconds; now when `None`), offline.
#[cfg(target_os = "macos")]
pub fn system_trust(chain: &[Vec<u8>], at: Option<i64>) -> Trust {
    use core_foundation::date::CFDate;
    use security_framework::certificate::SecCertificate;
    use security_framework::policy::SecPolicy;
    use security_framework::trust::SecTrust;

    let certs: Vec<SecCertificate> = chain
        .iter()
        .filter_map(|der| SecCertificate::from_der(der).ok())
        .collect();
    if certs.is_empty() {
        return Trust::NotTrusted("the signer's certificate cannot be read".into());
    }
    let policy = SecPolicy::create_x509();
    let mut trust = match SecTrust::create_with_certificates(&certs, &[policy]) {
        Ok(trust) => trust,
        Err(e) => return Trust::NotTrusted(e.to_string()),
    };
    // Never fetch missing intermediates or revocation data.
    let _ = trust.set_network_fetch_allowed(false);
    if let Some(at) = at {
        // CFAbsoluteTime counts from 2001-01-01.
        let date = CFDate::new((at - 978_307_200) as f64);
        let _ = trust.set_trust_verify_date(&date);
    }
    match trust.evaluate_with_error() {
        Ok(()) => Trust::Trusted,
        Err(error) => Trust::NotTrusted(error.description().to_string()),
    }
}

#[cfg(not(target_os = "macos"))]
pub fn system_trust(_chain: &[Vec<u8>], _at: Option<i64>) -> Trust {
    Trust::NotChecked
}

// ---------------------------------------------------------------------------
// Times

/// A PDF date string (`D:YYYYMMDDHHmmSSOHH'mm'`, any tail optional) as Unix
/// seconds.
pub fn pdf_date(text: &str) -> Option<i64> {
    let text = text.trim();
    let digits = text.strip_prefix("D:").unwrap_or(text);
    let num = |range: std::ops::Range<usize>, default: u32| -> Option<u32> {
        match digits.get(range) {
            Some(s) if s.bytes().all(|b| b.is_ascii_digit()) && !s.is_empty() => s.parse().ok(),
            Some(_) => None,
            None => Some(default),
        }
    };
    let year = num(0..4, 0)?;
    if digits.len() < 4 {
        return None;
    }
    let month = num(4..6, 1)?;
    let day = num(6..8, 1)?;
    let hour = num(8..10, 0)?;
    let minute = num(10..12, 0)?;
    let second = num(12..14, 0)?;
    let base = der::DateTime::new(
        year as u16,
        month as u8,
        day as u8,
        hour as u8,
        minute as u8,
        second.min(59) as u8,
    )
    .ok()?
    .unix_duration()
    .as_secs() as i64;
    // Offset: Z, or +HH'mm' / -HH'mm'.
    let tail = digits.get(14..).unwrap_or("");
    let offset = match tail.as_bytes().first() {
        Some(sign @ (b'+' | b'-')) => {
            let rest: String = tail[1..].chars().filter(char::is_ascii_digit).collect();
            let h: i64 = rest.get(0..2).and_then(|s| s.parse().ok()).unwrap_or(0);
            let m: i64 = rest.get(2..4).and_then(|s| s.parse().ok()).unwrap_or(0);
            let total = h * 3600 + m * 60;
            if *sign == b'+' {
                total
            } else {
                -total
            }
        }
        _ => 0,
    };
    Some(base - offset)
}

/// Unix seconds as `YYYY-MM-DD HH:MM UTC`.
pub fn format_time(secs: i64) -> String {
    let Ok(secs) = u64::try_from(secs) else {
        return "before 1970".into();
    };
    match der::DateTime::from_unix_duration(std::time::Duration::from_secs(secs)) {
        Ok(t) => format!(
            "{:04}-{:02}-{:02} {:02}:{:02} UTC",
            t.year(),
            t.month(),
            t.day(),
            t.hour(),
            t.minutes()
        ),
        Err(_) => "an unreadable time".into(),
    }
}

// ---------------------------------------------------------------------------
// One signature

/// Everything known about one signature's cryptography.
#[derive(Clone, Debug)]
pub struct CryptoReport {
    /// False when the SubFilter is one SpeedyF cannot check.
    pub supported: bool,
    pub intact: bool,
    pub problem: Option<String>,
    pub signer: Identity,
    /// Signer certificate first.
    pub chain: Vec<Vec<u8>>,
    pub signing_time: Option<i64>,
    /// Where `signing_time` came from: "timestamp", "signer" or "pdf".
    pub time_source: Option<&'static str>,
    pub timestamp: Option<TimestampCheck>,
    pub digest: Option<DigestAlg>,
}

/// Check the cryptography of `sig` in `bytes`. Coverage (whether the byte
/// range is whole) is the caller's: this checks what the range covers.
pub fn check_crypto(bytes: &[u8], sig: &SignatureInfo) -> CryptoReport {
    let mut report = CryptoReport {
        supported: true,
        intact: false,
        problem: None,
        signer: Identity::default(),
        chain: Vec::new(),
        signing_time: None,
        time_source: None,
        timestamp: None,
        digest: None,
    };
    let sub_filter = sig.sub_filter.as_deref().unwrap_or("");
    let Some(spans) = super::parse::signed_spans(bytes, &sig.byte_range) else {
        report.problem = Some("the byte range points outside the file".into());
        return report;
    };
    let Some(der) = trim_der(&sig.contents) else {
        report.problem = Some("the signature value is empty or unreadable".into());
        return report;
    };
    let pdf_time = sig.m.as_deref().and_then(pdf_date);

    match sub_filter {
        "adbe.pkcs7.detached" | "ETSI.CAdES.detached" | "adbe.pkcs7.sha1" => {
            let cms = if sub_filter == "adbe.pkcs7.sha1" {
                // The signed content is the SHA-1 of the byte range, carried
                // inside the SignedData.
                let mut cms = check_signed_data(der, Content::Encapsulated);
                let expected = DigestAlg::Sha1.digest(&spans);
                if cms.econtent.as_deref() != Some(expected.as_slice()) {
                    cms.digest_ok = false;
                    cms.problem = Some("the document's bytes do not match what was signed".into());
                }
                cms
            } else {
                check_signed_data(der, Content::Detached(&spans))
            };
            report.intact = cms.intact();
            report.problem = cms.problem.clone();
            report.digest = cms.digest;
            report.signer = cms
                .chain
                .first()
                .and_then(|c| Certificate::from_der(c).ok())
                .map(|c| Identity::of(&c))
                .unwrap_or_default();
            report.chain = cms.chain.clone();
            if let Some(token) = &cms.timestamp_token {
                report.timestamp = Some(check_timestamp(token, &[&cms.signature_value]));
            }
            let stamped = report
                .timestamp
                .as_ref()
                .filter(|t| t.valid)
                .and_then(|t| t.time);
            (report.signing_time, report.time_source) = if let Some(t) = stamped {
                (Some(t), Some("timestamp"))
            } else if let Some(t) = cms.signing_time {
                (Some(t), Some("signer"))
            } else {
                (pdf_time, pdf_time.map(|_| "pdf"))
            };
        }
        "ETSI.RFC3161" => {
            let ts = check_timestamp(der, &spans);
            report.intact = ts.valid;
            report.problem = ts.problem.clone();
            report.chain = ts.chain.clone();
            report.signer = ts
                .chain
                .first()
                .and_then(|c| Certificate::from_der(c).ok())
                .map(|c| Identity::of(&c))
                .unwrap_or_default();
            report.signing_time = ts.time;
            report.time_source = ts.time.map(|_| "timestamp");
            report.timestamp = Some(ts);
        }
        other => {
            report.supported = false;
            report.problem = Some(if other.is_empty() {
                "the signature does not say how it was made".into()
            } else {
                format!("{other} signatures cannot be checked")
            });
            report.signing_time = pdf_time;
            report.time_source = pdf_time.map(|_| "pdf");
            if let Some(cert) = sig.cert.first().and_then(|c| Certificate::from_der(c).ok()) {
                report.signer = Identity::of(&cert);
            }
        }
    }
    if report.signer.name.is_none() {
        report.signer.name = sig.name.clone();
    }
    report
}
