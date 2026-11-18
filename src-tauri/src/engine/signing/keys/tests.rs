use super::*;
use std::process::Command;

fn openssl() -> bool {
    Command::new("openssl")
        .arg("version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// An RSA identity made by openssl, exported as .p12 with `extra` pkcs12
/// options; `None` without the openssl tool.
fn openssl_p12(dir: &Path, name: &str, password: &str, extra: &[&str]) -> Option<Vec<u8>> {
    if !openssl() {
        eprintln!("skipping: no openssl");
        return None;
    }
    let key = dir.join(format!("{name}.key"));
    let crt = dir.join(format!("{name}.crt"));
    let p12 = dir.join(format!("{name}.p12"));
    let made = Command::new("openssl")
        .args([
            "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "30",
        ])
        .args([
            "-subj",
            "/CN=Grace Hopper/O=Navy/emailAddress=grace@example.org",
        ])
        .args(["-addext", "basicConstraints=critical,CA:FALSE"])
        .args([
            "-addext",
            "keyUsage=critical,digitalSignature,nonRepudiation",
        ])
        .arg("-keyout")
        .arg(&key)
        .arg("-out")
        .arg(&crt)
        .output()
        .unwrap();
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );
    let exported = Command::new("openssl")
        .args(["pkcs12", "-export", "-passout"])
        .arg(format!("pass:{password}"))
        .args(extra)
        .arg("-inkey")
        .arg(&key)
        .arg("-in")
        .arg(&crt)
        .arg("-out")
        .arg(&p12)
        .output()
        .unwrap();
    assert!(
        exported.status.success(),
        "{}",
        String::from_utf8_lossy(&exported.stderr)
    );
    Some(std::fs::read(p12).unwrap())
}

fn verifies(signer: &dyn CmsSigner) {
    use crate::engine::signing::sign::cms;
    let digest: [u8; 32] = Sha256::digest(b"document bytes").into();
    let parts = cms::build(
        signer,
        &digest,
        crate::engine::signing::sign::SubFilter::Cades,
        0,
    )
    .unwrap();
    cms::verify_detached(&parts.to_der().unwrap(), &digest).unwrap();
}

#[test]
fn a_self_signed_identity_describes_itself_and_can_sign() {
    let (pkcs8, cert) = make_self_signed("Ada Lovelace", "ada@example.org", "Analytical").unwrap();
    assert!(can_sign_documents(&cert));
    let dto = describe(std::slice::from_ref(&cert), "x".into(), "file", true).unwrap();
    assert_eq!(dto.name, "Ada Lovelace");
    assert_eq!(dto.email, "ada@example.org");
    assert_eq!(dto.organization, "Analytical");
    assert_eq!(dto.issuer, "Ada Lovelace");
    assert!(dto.self_signed);
    assert!(dto.not_after - dto.not_before > 4 * 365 * 86_400);
    let signer = SoftwareSigner::from_pkcs8(&pkcs8, vec![cert]).unwrap();
    assert_eq!(signer.alg(), SignAlg::EcdsaSha256);
    verifies(&signer);
    assert!(make_self_signed("  ", "", "").is_err());
}

#[test]
fn a_key_must_match_its_certificate() {
    let (pkcs8, _) = make_self_signed("One", "", "").unwrap();
    let (_, other) = make_self_signed("Two", "", "").unwrap();
    assert!(SoftwareSigner::from_pkcs8(&pkcs8, vec![other]).is_err());
}

#[test]
fn the_file_store_encrypts_and_needs_the_passphrase() {
    let dir = tempfile::tempdir().unwrap();
    let store_dir = dir.path().join("certificates");
    let made = create_self_signed(
        &store_dir,
        StoreKind::File,
        "Ada",
        "ada@example.org",
        "",
        Some("correct horse"),
    )
    .unwrap();
    assert_eq!(made.store, "file");
    assert!(made.needs_passphrase);
    // No passphrase is refused before anything is generated or stored.
    assert!(create_self_signed(&store_dir, StoreKind::File, "Ada", "", "", None).is_err());
    let listed = FileStore::new(&store_dir).list().unwrap();
    assert_eq!(listed, vec![made.clone()]);
    // The key is not in the file in the clear.
    let raw = std::fs::read_to_string(store_dir.join("identities.json")).unwrap();
    assert!(!raw.contains("PRIVATE"));
    assert!(signer(&store_dir, &made.id, Some("wrong")).is_err());
    assert!(signer(&store_dir, &made.id, None).is_err());
    let s = signer(&store_dir, &made.id, Some("correct horse")).unwrap();
    verifies(s.as_ref());
    remove(&store_dir, &made.id).unwrap();
    assert!(FileStore::new(&store_dir).list().unwrap().is_empty());
}

#[test]
fn imports_modern_and_legacy_p12_files_into_the_file_store() {
    let dir = tempfile::tempdir().unwrap();
    let store_dir = dir.path().join("certificates");
    let Some(modern) = openssl_p12(dir.path(), "modern", "pw", &[]) else {
        return;
    };
    let legacy = openssl_p12(
        dir.path(),
        "legacy",
        "pw",
        &[
            "-certpbe",
            "PBE-SHA1-3DES",
            "-keypbe",
            "PBE-SHA1-3DES",
            "-macalg",
            "sha1",
        ],
    )
    .unwrap();
    assert!(import_p12(&store_dir, StoreKind::File, &modern, "nope", Some("pass")).is_err());
    let a = import_p12(&store_dir, StoreKind::File, &modern, "pw", Some("pass")).unwrap();
    assert_eq!(a.name, "Grace Hopper");
    assert_eq!(a.email, "grace@example.org");
    let b = import_p12(&store_dir, StoreKind::File, &legacy, "pw", Some("pass")).unwrap();
    assert_ne!(a.id, b.id);
    let s = signer(&store_dir, &a.id, Some("pass")).unwrap();
    assert_eq!(s.alg(), SignAlg::RsaSha256);
    verifies(s.as_ref());
}

#[test]
fn signing_eligibility_reads_the_usages() {
    let dir = tempfile::tempdir().unwrap();
    if !openssl() {
        return;
    }
    let crt = dir.path().join("server.crt");
    let made = Command::new("openssl")
        .args([
            "req",
            "-x509",
            "-newkey",
            "ec",
            "-pkeyopt",
            "ec_paramgen_curve:P-256",
            "-nodes",
            "-days",
            "2",
        ])
        .args([
            "-subj",
            "/CN=www.example.org",
            "-addext",
            "extendedKeyUsage=serverAuth",
        ])
        .arg("-keyout")
        .arg(dir.path().join("server.key"))
        .arg("-out")
        .arg(&crt)
        .args(["-outform", "DER"])
        .output()
        .unwrap();
    assert!(made.status.success());
    assert!(!can_sign_documents(&std::fs::read(crt).unwrap()));
}

/// The Keychain path, against a temporary keychain file that is deleted
/// afterwards: the login keychain is never touched.
#[cfg(target_os = "macos")]
#[test]
fn keychain_identities_import_list_and_sign_in_a_temporary_keychain() {
    use security_framework::os::macos::keychain::CreateOptions;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("speedyf-test.keychain-db");
    let kc = CreateOptions::new()
        .password("speedyf-test")
        .prompt_user(false)
        .create(&path)
        .expect("temporary keychain");
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = Command::new("security")
                .arg("delete-keychain")
                .arg(&self.0)
                .output();
        }
    }
    let _cleanup = Cleanup(path.clone());
    let only = std::slice::from_ref(&kc);

    assert!(keychain::list(Some(only)).unwrap().is_empty());
    let (pkcs8, cert) = make_self_signed("Keychain Person", "kp@example.org", "").unwrap();
    let added = keychain::add(&pkcs8, vec![cert], "Keychain Person", Some(&kc)).unwrap();
    assert_eq!(added.store, "keychain");
    assert!(!added.needs_passphrase);
    let listed = keychain::list(Some(only)).unwrap();
    assert!(listed.iter().any(|d| d.id == added.id));
    let s = keychain::signer(&added.id, Some(only)).unwrap();
    assert_eq!(s.alg(), SignAlg::EcdsaSha256);
    verifies(&s);

    if let Some(p12) = openssl_p12(dir.path(), "kc", "pw", &[]) {
        let imported = keychain::import_p12(&p12, "pw", Some(&kc)).unwrap();
        assert_eq!(imported.name, "Grace Hopper");
        let s = keychain::signer(&imported.id, Some(only)).unwrap();
        assert_eq!(s.alg(), SignAlg::RsaSha256);
        verifies(&s);
        assert!(keychain::import_p12(&p12, "wrong", Some(&kc)).is_err());
    }
}
