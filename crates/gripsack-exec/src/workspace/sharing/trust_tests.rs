use super::*;
use std::os::unix::fs::PermissionsExt;

fn hex<const N: usize>(value: &str) -> [u8; N] {
    assert_eq!(value.len(), N * 2);
    std::array::from_fn(|index| u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).unwrap())
}
fn seed() -> [u8; 32] {
    hex("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60")
}
fn policy_value() -> serde_json::Value {
    serde_json::json!({"version":1,"keys":[{"key_id":"publisher-key","publisher":"alice","namespaces":["tools"],
        "public_key":STANDARD.encode(hex::<32>("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a")),
        "allowed_layouts":["relocatable"],"required_validators":["layout-safety"]}],"revoked_keys":[],"revoked_manifests":[]})
}
fn policy(value: &serde_json::Value) -> TrustPolicy {
    TrustPolicy::from_bytes(&serde_json::to_vec(value).unwrap()).unwrap()
}
fn envelope(manifest: &[u8]) -> Vec<u8> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("key");
    std::fs::write(&path, seed()).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    sign(manifest, &path, "publisher-key").unwrap()
}

#[test]
fn independent_rfc8032_ed25519_vector_one() {
    // RFC 8032 section 7.1 TEST 1, not a signature made by this implementation.
    let public = hex::<32>("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a");
    let signature = hex::<64>(concat!(
        "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555f",
        "b8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b"
    ));
    VerifyingKey::from_bytes(&public)
        .unwrap()
        .verify_strict(b"", &Signature::from_bytes(&signature))
        .unwrap();
    assert!(
        VerifyingKey::from_bytes(&public)
            .unwrap()
            .verify_strict(b"tampered", &Signature::from_bytes(&signature))
            .is_err()
    );
}
#[test]
fn dsse_standard_preauthentication_encoding() {
    assert_eq!(
        preauth(b"{}"),
        b"DSSEv1 41 application/vnd.gripsack.portable.v1+json 2 {}".to_vec()
    );
}
#[test]
fn exact_bytes_scope_and_layout_are_separate_authorizations() {
    let manifest = b"{}";
    let verified = policy(&policy_value())
        .verify(manifest, &envelope(manifest))
        .unwrap();
    assert!(verified.authorize("tools", "alice").is_ok());
    assert!(verified.authorize("upstream", "alice").is_err());
    assert!(verified.authorize("tools", "mallory").is_err());
    assert!(
        verified
            .authorize_requested_layouts(&["relocatable"])
            .is_ok()
    );
    assert!(verified.authorize_requested_layouts(&["fixed"]).is_err());
    assert!(
        verified
            .authorize_layouts(&["relocatable"], &["layout-safety"])
            .is_ok()
    );
    assert!(
        verified
            .authorize_layouts(&["fixed"], &["layout-safety"])
            .is_err()
    );
    assert!(verified.authorize_layouts(&["relocatable"], &[]).is_err());
    assert!(
        policy(&policy_value())
            .verify(b"{ }", &envelope(manifest))
            .is_err()
    );
}
#[test]
fn tampering_type_signature_unknown_fields_and_duplicates_reject() {
    let original = envelope(b"{}");
    for (field, value) in [("payloadType", "wrong"), ("payload", "e30K")] {
        let mut changed: serde_json::Value = serde_json::from_slice(&original).unwrap();
        changed[field] = value.into();
        assert!(
            policy(&policy_value())
                .verify(b"{}", &serde_json::to_vec(&changed).unwrap())
                .is_err()
        );
    }
    let mut changed: serde_json::Value = serde_json::from_slice(&original).unwrap();
    changed["signatures"][0]["sig"] = STANDARD.encode([0u8; 64]).into();
    assert!(
        policy(&policy_value())
            .verify(b"{}", &serde_json::to_vec(&changed).unwrap())
            .is_err()
    );
    changed["algorithm"] = "none".into();
    assert!(
        policy(&policy_value())
            .verify(b"{}", &serde_json::to_vec(&changed).unwrap())
            .is_err()
    );
    let duplicate =
        String::from_utf8(original)
            .unwrap()
            .replacen("{", "{\"payloadType\":\"wrong\",", 1);
    assert!(
        policy(&policy_value())
            .verify(b"{}", duplicate.as_bytes())
            .is_err()
    );
}
#[test]
fn local_revocations_rotation_and_revision_are_rechecked() {
    let signature = envelope(b"{}");
    let original = policy_value();
    let mut revoked = original.clone();
    revoked["revoked_keys"] = serde_json::json!(["publisher-key"]);
    assert!(policy(&revoked).verify(b"{}", &signature).is_err());
    revoked = original.clone();
    revoked["revoked_manifests"] = serde_json::json!([digest(b"{}")]);
    assert!(policy(&revoked).verify(b"{}", &signature).is_err());
    let mut rotated = original.clone();
    let mut second = rotated["keys"][0].clone();
    second["key_id"] = "next-key".into();
    rotated["keys"].as_array_mut().unwrap().push(second);
    assert!(policy(&rotated).verify(b"{}", &signature).is_ok());
    assert_ne!(policy(&rotated).revision(), policy(&original).revision());
    rotated["keys"].as_array_mut().unwrap().remove(0);
    assert!(policy(&rotated).verify(b"{}", &signature).is_err());
}
#[test]
fn key_file_permissions_length_and_symlinks_reject() {
    let dir = tempfile::tempdir().unwrap();
    let key = dir.path().join("key");
    std::fs::write(&key, seed()).unwrap();
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(sign(b"{}", &key, "publisher-key").is_err());
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&key, &link).unwrap();
    assert!(sign(b"{}", &link, "publisher-key").is_err());
    std::fs::write(&key, [0; 31]).unwrap();
    assert!(sign(b"{}", &key, "publisher-key").is_err());
}
#[test]
fn unknown_validator_and_weak_public_key_fail_closed() {
    let mut value = policy_value();
    value["keys"][0]["required_validators"] = serde_json::json!(["run-publisher-hook"]);
    assert!(TrustPolicy::from_bytes(&serde_json::to_vec(&value).unwrap()).is_err());
    value = policy_value();
    value["keys"][0]["public_key"] = STANDARD.encode([0u8; 32]).into();
    assert!(TrustPolicy::from_bytes(&serde_json::to_vec(&value).unwrap()).is_err());
    assert!(
        policy(&policy_value())
            .verify(&vec![0; MAX_MANIFEST_BYTES + 1], b"{}")
            .is_err()
    );
}
