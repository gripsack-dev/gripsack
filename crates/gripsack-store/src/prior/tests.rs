use super::*;

#[test]
fn historical_prior_wire_admits_only_hashes_and_permission_bits() {
    let valid = format!(
        r#"{{"kind":"file","hash":"{}","mode":4095}}"#,
        "A0".repeat(32)
    );
    let prior: Prior = serde_json::from_str(&valid).unwrap();
    let Prior::File { hash, mode } = prior else {
        panic!("expected saved file")
    };
    assert_eq!(hash.as_str(), "A0".repeat(32));
    assert_eq!(mode.bits(), 0o7777);
    for invalid in [
        r#"{"kind":"file","hash":"../outside","mode":420}"#.to_string(),
        format!(
            r#"{{"kind":"file","hash":"{}","mode":4096}}"#,
            "ab".repeat(32)
        ),
        format!(
            r#"{{"kind":"file","hash":"{}","mode":-1}}"#,
            "ab".repeat(32)
        ),
        format!(
            r#"{{"kind":"file","hash":"{}","mode":420}}"#,
            "z0".repeat(32)
        ),
    ] {
        assert!(
            serde_json::from_str::<Prior>(&invalid).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn captured_blob_is_private_and_corrupt_bytes_cannot_restore() {
    let temporary = tempfile::tempdir().unwrap();
    let home = gripsack_fs::open(temporary.path()).unwrap();
    let identity = store_blob(&home, b"abc").unwrap();
    assert_eq!(
        identity.as_str(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(read_blob(&home, &identity).unwrap(), b"abc");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(identity.path_in(temporary.path()))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(temporary.path().join("prior"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    std::fs::write(identity.path_in(temporary.path()), b"corrupt").unwrap();
    assert_eq!(
        read_blob(&home, &identity).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    // Capturing the genuine original again quarantines the corrupt object and
    // re-establishes the admitted prior without changing its wire identity.
    assert_eq!(store_blob(&home, b"abc").unwrap(), identity);
    assert_eq!(read_blob(&home, &identity).unwrap(), b"abc");
}
