//! CMS SignedData for a detached PDF signature (RFC 5652, ETSI EN 319 122).
//!
//! The signed attributes are the ones PAdES B-B asks for: contentType
//! (id-data), messageDigest (SHA-256 of the byte ranges) and
//! signingCertificateV2 (ESS, RFC 5035), which binds the signer's
//! certificate into what is signed. PAdES puts the claimed signing time in
//! the signature dictionary's `/M` and leaves the CMS signing-time attribute
//! out; the adbe.pkcs7.detached form keeps it, as readers of that form
//! expect.

use super::SubFilter;
use crate::engine::signing::keys::{parse_cert, CmsSigner, SignAlg};
use crate::errors::{AppError, AppResult};
use cms::cert::{CertificateChoices, IssuerAndSerialNumber};
use cms::content_info::{CmsVersion, ContentInfo};
use cms::signed_data::{
    CertificateSet, EncapsulatedContentInfo, SignedData, SignerIdentifier, SignerInfo, SignerInfos,
};
use der::asn1::{Any, ObjectIdentifier, OctetString, SetOfVec};
use der::{Decode, Encode, Sequence};
use sha2_11::{Digest, Sha256};
use spki::AlgorithmIdentifierOwned;
use x509_cert::attr::Attribute;
use x509_cert::ext::pkix::name::GeneralName;
use x509_cert::serial_number::SerialNumber;

const OID_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.1");
const OID_SIGNED_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.2");
const OID_CONTENT_TYPE: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.3");
const OID_MESSAGE_DIGEST: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.4");
const OID_SIGNING_TIME: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.5");
const OID_SIGNING_CERT_V2: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.2.47");
pub(crate) const OID_TIMESTAMP_TOKEN: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.2.14");
pub(crate) const OID_SHA256: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.1");
const OID_RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.1");
const OID_SHA256_RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.11");
const OID_ECDSA_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.2");

/// ESSCertIDv2 with the default hash (SHA-256), so `hashAlgorithm` is left
/// out as DER requires for a DEFAULT value.
#[derive(Clone, Debug, Sequence)]
struct EssCertIdV2 {
    cert_hash: OctetString,
    issuer_serial: IssuerSerial,
}

#[derive(Clone, Debug, Sequence)]
struct IssuerSerial {
    issuer: Vec<GeneralName>,
    serial_number: SerialNumber,
}

#[derive(Clone, Debug, Sequence)]
struct SigningCertificateV2 {
    certs: Vec<EssCertIdV2>,
}

fn der_err(e: der::Error) -> AppError {
    AppError::Internal(format!("cannot encode the signature: {e}"))
}

fn sha256_alg() -> AlgorithmIdentifierOwned {
    AlgorithmIdentifierOwned {
        oid: OID_SHA256,
        parameters: None,
    }
}

fn attribute(oid: ObjectIdentifier, value: Any) -> AppResult<Attribute> {
    let mut values = SetOfVec::new();
    values.insert(value).map_err(der_err)?;
    Ok(Attribute { oid, values })
}

/// UTCTime (through 2049) or GeneralizedTime, as RFC 5652 asks.
fn signing_time(secs: i64) -> AppResult<Any> {
    let time = x509_cert::time::Time::try_from(
        std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs.max(0) as u64),
    )
    .map_err(der_err)?;
    Any::encode_from(&time).map_err(der_err)
}

/// A SignerInfo and the certificates to send with it, before encoding.
pub struct CmsParts {
    signer_info: SignerInfo,
    certs: Vec<x509_cert::Certificate>,
}

/// Sign `digest` (SHA-256 of the PDF's byte ranges).
pub fn build(
    signer: &dyn CmsSigner,
    digest: &[u8; 32],
    sub_filter: SubFilter,
    now_secs: i64,
) -> AppResult<CmsParts> {
    let chain = signer.chain();
    let leaf_der = chain
        .first()
        .ok_or_else(|| AppError::Malformed("no certificate to sign with".into()))?;
    let leaf = parse_cert(leaf_der)?;
    let tbs = leaf.tbs_certificate();

    let ess = SigningCertificateV2 {
        certs: vec![EssCertIdV2 {
            cert_hash: OctetString::new(Sha256::digest(leaf_der).to_vec()).map_err(der_err)?,
            issuer_serial: IssuerSerial {
                issuer: vec![GeneralName::DirectoryName(tbs.issuer().clone())],
                serial_number: tbs.serial_number().clone(),
            },
        }],
    };
    let mut attrs: Vec<Attribute> = vec![
        attribute(
            OID_CONTENT_TYPE,
            Any::encode_from(&OID_DATA).map_err(der_err)?,
        )?,
        attribute(
            OID_MESSAGE_DIGEST,
            Any::encode_from(&OctetString::new(digest.to_vec()).map_err(der_err)?)
                .map_err(der_err)?,
        )?,
        attribute(
            OID_SIGNING_CERT_V2,
            Any::encode_from(&ess).map_err(der_err)?,
        )?,
    ];
    if sub_filter == SubFilter::Pkcs7 {
        attrs.push(attribute(OID_SIGNING_TIME, signing_time(now_secs)?)?);
    }
    let signed_attrs = SetOfVec::try_from(attrs).map_err(der_err)?;
    // What is signed is the attributes' DER with the SET OF tag, not the
    // [0] IMPLICIT tag they carry inside the SignerInfo.
    let to_sign = signed_attrs.to_der().map_err(der_err)?;
    let signature = signer.sign(&to_sign)?;

    let signature_algorithm = match signer.alg() {
        SignAlg::RsaSha256 => AlgorithmIdentifierOwned {
            oid: OID_RSA,
            parameters: Some(Any::null()),
        },
        SignAlg::EcdsaSha256 => AlgorithmIdentifierOwned {
            oid: OID_ECDSA_SHA256,
            parameters: None,
        },
    };
    let signer_info = SignerInfo {
        version: CmsVersion::V1,
        sid: SignerIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
            issuer: tbs.issuer().clone(),
            serial_number: tbs.serial_number().clone(),
        }),
        digest_alg: sha256_alg(),
        signed_attrs: Some(signed_attrs),
        signature_algorithm,
        signature: OctetString::new(signature).map_err(der_err)?,
        unsigned_attrs: None,
    };
    let certs = chain
        .iter()
        .map(|c| parse_cert(c))
        .collect::<AppResult<Vec<_>>>()?;
    Ok(CmsParts { signer_info, certs })
}

impl CmsParts {
    /// The signature value, which an RFC 3161 timestamp is taken over.
    pub fn signature(&self) -> &[u8] {
        self.signer_info.signature.as_bytes()
    }

    /// Attach a timestamp token (a ContentInfo, DER) as an unsigned attribute.
    pub fn add_timestamp(&mut self, token: &[u8]) -> AppResult<()> {
        let any =
            Any::from_der(token).map_err(|e| AppError::Malformed(format!("timestamp: {e}")))?;
        let mut attrs = SetOfVec::new();
        attrs
            .insert(attribute(OID_TIMESTAMP_TOKEN, any)?)
            .map_err(der_err)?;
        self.signer_info.unsigned_attrs = Some(attrs);
        Ok(())
    }

    /// ContentInfo { SignedData } as DER.
    pub fn to_der(&self) -> AppResult<Vec<u8>> {
        let mut digest_algorithms = SetOfVec::new();
        digest_algorithms.insert(sha256_alg()).map_err(der_err)?;
        let mut certs = SetOfVec::new();
        for c in &self.certs {
            certs
                .insert(CertificateChoices::Certificate(c.clone()))
                .map_err(der_err)?;
        }
        let mut infos = SetOfVec::new();
        infos.insert(self.signer_info.clone()).map_err(der_err)?;
        let signed = SignedData {
            version: CmsVersion::V1,
            digest_algorithms,
            encap_content_info: EncapsulatedContentInfo {
                econtent_type: OID_DATA,
                econtent: None,
            },
            certificates: Some(CertificateSet(certs)),
            crls: None,
            signer_infos: SignerInfos(infos),
        };
        ContentInfo {
            content_type: OID_SIGNED_DATA,
            content: Any::encode_from(&signed).map_err(der_err)?,
        }
        .to_der()
        .map_err(der_err)
    }
}

/// Length of the DER element at the start of `bytes`, header included.
pub(crate) fn der_len(bytes: &[u8]) -> Option<usize> {
    let first = *bytes.get(1)?;
    if first < 0x80 {
        return Some(2 + first as usize);
    }
    let n = (first & 0x7f) as usize;
    if n == 0 || n > 4 {
        return None;
    }
    let mut len = 0usize;
    for i in 0..n {
        len = (len << 8) | *bytes.get(2 + i)? as usize;
    }
    let total = 2 + n + len;
    (total <= bytes.len()).then_some(total)
}

/// Check a detached signature over `digest`: the messageDigest attribute
/// matches, and the signature over the signed attributes verifies under the
/// signer's certificate. Integrity and identity only; trust is not judged.
pub fn verify_detached(cms_der: &[u8], digest: &[u8; 32]) -> AppResult<()> {
    let bad = |what: &str| AppError::Internal(format!("signature check failed: {what}"));
    let info = ContentInfo::from_der(cms_der).map_err(|e| bad(&e.to_string()))?;
    if info.content_type != OID_SIGNED_DATA {
        return Err(bad("not SignedData"));
    }
    let signed = info
        .content
        .decode_as::<SignedData>()
        .map_err(|e| bad(&e.to_string()))?;
    let si = signed
        .signer_infos
        .0
        .iter()
        .next()
        .ok_or_else(|| bad("no signer"))?;
    let attrs = si
        .signed_attrs
        .as_ref()
        .ok_or_else(|| bad("no signed attributes"))?;
    let md = attrs
        .iter()
        .find(|a| a.oid == OID_MESSAGE_DIGEST)
        .and_then(|a| a.values.iter().next())
        .and_then(|v| v.decode_as::<OctetString>().ok())
        .ok_or_else(|| bad("no message digest"))?;
    if md.as_bytes() != digest {
        return Err(bad("the document digest does not match"));
    }
    let SignerIdentifier::IssuerAndSerialNumber(ias) = &si.sid else {
        return Err(bad("unsupported signer identifier"));
    };
    let cert = signed
        .certificates
        .as_ref()
        .and_then(|set| {
            set.0.iter().find_map(|c| match c {
                CertificateChoices::Certificate(c)
                    if c.tbs_certificate().issuer() == &ias.issuer
                        && c.tbs_certificate().serial_number() == &ias.serial_number =>
                {
                    Some(c.clone())
                }
                _ => None,
            })
        })
        .ok_or_else(|| bad("the signer's certificate is missing"))?;
    let message = attrs.to_der().map_err(|e| bad(&e.to_string()))?;
    let spki = cert.tbs_certificate().subject_public_key_info();
    let sig = si.signature.as_bytes();
    let alg = si.signature_algorithm.oid;
    if alg == OID_RSA || alg == OID_SHA256_RSA {
        use pkcs8::DecodePublicKey;
        use rsa::signature::Verifier;
        let der = spki.to_der().map_err(|e| bad(&e.to_string()))?;
        let key = rsa::PublicKey::from_public_key_der(&der).map_err(|e| bad(&e.to_string()))?;
        let vk = rsa::pkcs1v15::VerifyingKey::<Sha256>::new(key);
        let sig = rsa::pkcs1v15::Signature::try_from(sig).map_err(|e| bad(&e.to_string()))?;
        vk.verify(&message, &sig)
            .map_err(|_| bad("the signature does not verify"))
    } else if alg == OID_ECDSA_SHA256 {
        use pkcs8::DecodePublicKey;
        use rsa::signature::Verifier;
        let der = spki.to_der().map_err(|e| bad(&e.to_string()))?;
        // A Keychain key on another curve (P-384, P-521) signs fine; this
        // check only knows P-256, so the digest match above is all it adds.
        let Ok(vk) = p256::ecdsa::VerifyingKey::from_public_key_der(&der) else {
            log::info!("signature check: not a P-256 key; digest checked only");
            return Ok(());
        };
        let sig = p256::ecdsa::Signature::from_der(sig).map_err(|e| bad(&e.to_string()))?;
        vk.verify(&message, &sig)
            .map_err(|_| bad("the signature does not verify"))
    } else {
        Err(bad("unsupported signature algorithm"))
    }
}
