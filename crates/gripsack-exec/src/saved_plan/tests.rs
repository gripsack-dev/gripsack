use super::*;

#[test]
fn duplicate_and_unknown_envelope_fields_are_rejected() {
    assert!(SavedPlan::decode(br#"{"version":1,"version":1,"digest":"x","binding":{}}"#).is_err());
    assert!(SavedPlan::decode(br#"{"version":1,"digest":"x","binding":{},"force":true}"#).is_err());
}

#[test]
fn oversized_and_deep_envelopes_are_rejected() {
    let bytes = vec![b' '; MAX_BYTES as usize + 1];
    assert!(SavedPlan::decode(&bytes).is_err());
    let nested = format!("{}0{}", "[".repeat(65), "]".repeat(65));
    assert!(SavedPlan::decode(nested.as_bytes()).is_err());
}

#[test]
fn exact_binding_changes_are_never_unknown_equality() {
    for subject in [
        "source",
        "lock",
        "base generation",
        "destination",
        "trust",
        "job revision",
        "selected scope",
    ] {
        let error = require_equal(&Some("reviewed"), &Some("changed"), subject).unwrap_err();
        assert!(error.to_string().contains(subject));
        assert!(require_equal(&Some("reviewed"), &None, subject).is_err());
        assert!(require_equal(&Some("reviewed"), &Some("reviewed"), subject).is_ok());
    }
}

#[test]
fn exact_digest_changes_with_reviewed_bytes() {
    assert_ne!(
        digest(b"same semantic content\n"),
        digest(b"same semantic content")
    );
    assert_eq!(
        digest(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

fn binding() -> Binding {
    let ir = serde_json::json!({
        "ir_version": 10,
        "host": {"os": "linux", "arch": "x86_64", "tags": []},
        "workspace": {"span": {"file": "gripsack.ts", "line": 1}, "outputs": [{
            "kind": "profile", "name": "personal",
            "span": {"file": "gripsack.ts", "line": 2}, "files": []
        }]}
    });
    serde_json::from_value(serde_json::json!({
        "repository": "/repo", "home": "/state",
        "source": "captured-source", "policy": "captured-policy",
        "evaluation": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "host": "host", "selected": [], "frontend": "/frontend",
        "pinned_frontend": null, "exclusions": [], "ir": ir.to_string(),
        "facts": {}, "probes": {},
        "state": {"base": null, "manifest": null, "trust": null,
            "sharing_trust": null, "activation": null, "operations": [],
            "packages": [], "schedules": null}
    }))
    .unwrap()
}

#[test]
fn digest_admission_rejects_tampered_source_lock_base_scope_and_trust() {
    let binding = binding();
    let envelope = Envelope {
        version: FORMAT_VERSION,
        digest: digest(&serde_json::to_vec(&binding).unwrap()),
        binding,
    };
    let bytes = serde_json::to_vec(&envelope).unwrap();
    assert!(SavedPlan::decode(&bytes).is_ok());
    for path in [
        "/binding/source",
        "/binding/policy",
        "/binding/state/base",
        "/binding/state/trust",
        "/binding/host",
        "/binding/ir",
    ] {
        let mut changed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        *changed.pointer_mut(path).unwrap() = serde_json::json!("tampered");
        assert!(
            SavedPlan::decode(&serde_json::to_vec(&changed).unwrap()).is_err(),
            "{path}"
        );
    }
    let mut changed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    changed["version"] = serde_json::json!(FORMAT_VERSION + 1);
    assert!(SavedPlan::decode(&serde_json::to_vec(&changed).unwrap()).is_err());
}

#[test]
fn logical_locations_round_trip_quoted_roots_without_rewriting_literal_content() {
    use store::source_bundle::{SourceBundle, SourceCapturePolicy};
    for name in ["quoted\"repository", "backslash\\repository"] {
        let temporary = tempfile::tempdir().unwrap();
        let repo = temporary.path().join(name);
        let frontend = temporary.path().join("frontend");
        let home = temporary.path().join("home");
        for directory in [&repo, &frontend, &home] {
            std::fs::create_dir(directory).unwrap();
        }
        std::fs::write(repo.join("gripsack.ts"), "// fixture\n").unwrap();
        std::fs::write(frontend.join("driver.ts"), "// frontend\n").unwrap();
        let sources = SourceBundle::capture(
            &repo,
            &frontend,
            None,
            &home,
            SourceCapturePolicy::default(),
        )
        .unwrap();
        let captured = sources
            .repository()
            .join("gripsack.ts")
            .to_str()
            .unwrap()
            .to_owned();
        let mut wire: serde_json::Value = serde_json::from_str(&binding().ir).unwrap();
        wire["workspace"]["span"]["file"] = captured.clone().into();
        wire["workspace"]["outputs"][0]["span"]["file"] = captured.clone().into();
        wire["workspace"]["outputs"][0]["files"] = serde_json::json!([{
            "span": {"file": captured, "line": 3},
            "content": {"kind": "literal", "text": captured},
            "destination": {"kind": "tracked_copy", "path": "~/.reviewed"}
        }]);
        let mut ir = admit_ir(&wire.to_string()).unwrap();
        crate::rebind_captured_ir(&mut ir, &sources).unwrap();
        let serialized = serde_json::to_string(&ir).unwrap();
        let admitted = admit_ir(&serialized).unwrap();
        assert_eq!(
            admitted.workspace.span.file,
            repo.join("gripsack.ts").to_str().unwrap()
        );
        let gripsack_ir::workspace_model::WorkspaceOutput::Profile(profile) =
            &admitted.workspace.outputs[0]
        else {
            panic!("profile");
        };
        let gripsack_ir::workspace_model::WorkspaceContent::Literal { text } =
            &profile.files[0].content
        else {
            panic!("literal");
        };
        assert_eq!(
            text, &captured,
            "literal configuration is not a path-rebinding field"
        );
    }
}
