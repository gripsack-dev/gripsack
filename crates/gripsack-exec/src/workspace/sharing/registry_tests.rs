use super::*;
use std::{
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    thread,
};

#[test]
fn expired_session_rejects_before_network_request() {
    let mut registry = Registry::new(
        "http://127.0.0.1:1/",
        "tools/app",
        RegistryAuth::Anonymous,
        true,
    )
    .unwrap();
    registry.deadline = Instant::now() - Duration::from_secs(1);
    let error = match registry.request(reqwest::Method::GET, registry.url("blobs/test").unwrap()) {
        Ok(_) => panic!("expired session created a request"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("session deadline exceeded"));
}

struct Reply {
    status: &'static str,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}
fn reply(status: &'static str, body: Vec<u8>) -> Reply {
    Reply {
        status,
        headers: Vec::new(),
        body,
    }
}
type RecordedRequests = Vec<(String, Vec<u8>)>;
fn server(replies: Vec<Reply>) -> (String, thread::JoinHandle<RecordedRequests>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}/", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let mut requests = Vec::new();
        for reply in replies {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut start = String::new();
            reader.read_line(&mut start).unwrap();
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap();
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            requests.push((start.trim().into(), body));
            write!(
                stream,
                "HTTP/1.1 {}\r\nContent-Length: {}\r\nConnection: close\r\n",
                reply.status,
                reply.body.len()
            )
            .unwrap();
            for (key, value) in reply.headers {
                write!(stream, "{key}: {value}\r\n").unwrap();
            }
            stream.write_all(b"\r\n").unwrap();
            stream.write_all(&reply.body).unwrap();
        }
        requests
    });
    (origin, handle)
}
fn fixture_manifest() -> super::super::format::PortableManifest {
    serde_json::from_value(serde_json::json!({
        "version": 1, "namespace": "tools/app", "publisher": "fixture",
        "target": {"os": "linux", "arch": "x86_64", "abi": {"kind": "linux"},
            "minimum_kernel": "3.2", "cpu": "x86_64"},
        "layout": {"kind": "relocatable", "contract": {"behavior": "transport fixture"}},
        "components": [{
            "id": "app", "payload": {"sha256": digest(b"payload"), "size": 7,
                "media_type": super::super::format::TREE_MEDIA_TYPE},
            "tree": "b".repeat(64), "runtime": [], "conda": null, "external": null,
            "inventory": []
        }],
        "roots": ["app"], "commands": {}, "environment": {}, "licenses": [],
        "redistribution_authorized": true
    }))
    .unwrap()
}

fn bundle() -> RegistryBundle {
    RegistryBundle {
        manifest: fixture_manifest().encode().unwrap(),
        envelope: b"envelope".to_vec(),
        external_manifests: BTreeMap::new(),
        blobs: BTreeMap::from([(digest(b"payload"), b"payload".to_vec())]),
    }
}

fn external_bundle() -> RegistryBundle {
    use super::super::format::{Component, ExternalReference};
    let mut child = fixture_manifest();
    child.namespace = "upstream/library".into();
    child.publisher = "upstream".into();
    let manifest = child.encode().unwrap();
    let identity = digest(&manifest);
    let mut root = fixture_manifest();
    root.components[0].runtime.push("library".into());
    root.components.push(Component {
        id: "library".into(),
        payload: None,
        tree: None,
        conda: None,
        external: Some(ExternalReference {
            manifest: identity.clone(),
            namespace: child.namespace,
            publisher: child.publisher,
        }),
        runtime: vec![],
        inventory: vec![],
    });
    RegistryBundle {
        manifest: root.encode().unwrap(),
        envelope: b"root signature".to_vec(),
        external_manifests: BTreeMap::from([(
            identity,
            ExternalManifest {
                manifest,
                envelope: b"upstream signature".to_vec(),
            },
        )]),
        // Root and dependency intentionally share the same payload object.
        blobs: BTreeMap::from([(digest(b"payload"), b"payload".to_vec())]),
    }
}
#[test]
fn transport_requires_scoped_explicit_security_configuration() {
    assert!(
        Registry::new(
            "http://registry.example/",
            "tools/app",
            RegistryAuth::Anonymous,
            true
        )
        .is_err()
    );
    assert!(
        Registry::new(
            "http://127.0.0.1/",
            "tools/app",
            RegistryAuth::Anonymous,
            false
        )
        .is_err()
    );
    assert!(
        Registry::new(
            "http://127.0.0.1/",
            "tools/app",
            RegistryAuth::Bearer("secret".into()),
            true
        )
        .is_err()
    );
    assert!(
        Registry::new(
            "https://user:secret@registry.example/",
            "tools/app",
            RegistryAuth::Anonymous,
            false
        )
        .is_err()
    );
    assert!(
        Registry::new(
            "https://registry.example/",
            "../app",
            RegistryAuth::Anonymous,
            false
        )
        .is_err()
    );
    assert!(require_digest("latest").is_err());
    assert!(require_digest(&format!("sha256:{}", "A".repeat(64))).is_err());
}
#[test]
fn pushes_blobs_before_oci_manifest_and_preserves_envelope_bytes() {
    let mut replies = Vec::new();
    for _ in 0..4 {
        let mut begin = reply("202 Accepted", vec![]);
        begin.headers.push((
            "Location".into(),
            "/v2/tools/app/blobs/uploads/session?state=opaque".into(),
        ));
        replies.push(begin);
        replies.push(reply("201 Created", vec![]));
    }
    replies.push(reply("201 Created", vec![]));
    let (origin, handle) = server(replies);
    let registry = Registry::new(&origin, "tools/app", RegistryAuth::Anonymous, true).unwrap();
    let transport_digest = registry.push("release", bundle()).unwrap();
    let requests = handle.join().unwrap();
    assert_eq!(requests.len(), 9);
    assert_eq!(
        requests.last().unwrap().0,
        "PUT /v2/tools/app/manifests/release HTTP/1.1"
    );
    let transport: TransportManifest = serde_json::from_slice(&requests[8].1).unwrap();
    assert_eq!(
        transport_digest,
        format!("sha256:{}", digest(&requests[8].1))
    );
    assert_eq!(requests[5].1, b"envelope");
    assert_eq!(transport.layers[1].media_type, ENVELOPE_TYPE);
    assert!(
        requests[..8]
            .iter()
            .all(|(line, _)| line.contains("/blobs/uploads/"))
    );
}
#[test]
fn digest_bound_pull_checks_payload_and_outer_manifest() {
    let bundle = bundle();
    let transport = TransportManifest {
        schema_version: 2,
        media_type: OCI_MANIFEST.into(),
        artifact_type: ARTIFACT_TYPE.into(),
        config: Descriptor::new(EMPTY_CONFIG_TYPE, b"{}"),
        layers: vec![
            Descriptor::new(PAYLOAD_TYPE, &bundle.manifest),
            Descriptor::new(ENVELOPE_TYPE, &bundle.envelope),
            Descriptor::new(PAYLOAD_BLOB_TYPE, b"payload"),
        ],
    };
    let encoded = serde_json::to_vec(&transport).unwrap();
    let reference = format!("sha256:{}", digest(&encoded));
    let mut outer = reply("200 OK", encoded);
    outer
        .headers
        .push(("Content-Type".into(), OCI_MANIFEST.into()));
    let (origin, handle) = server(vec![
        outer,
        reply("200 OK", b"{}".to_vec()),
        reply("200 OK", bundle.manifest),
        reply("200 OK", bundle.envelope),
        reply("200 OK", b"payload".to_vec()),
    ]);
    let pulled = Registry::new(&origin, "tools/app", RegistryAuth::Anonymous, true)
        .unwrap()
        .pull(&reference)
        .unwrap();
    assert_eq!(pulled.blobs[&digest(b"payload")], b"payload");
    assert_eq!(handle.join().unwrap().len(), 5);
    let mut bad = reply("200 OK", b"tampered".to_vec());
    bad.headers
        .push(("Content-Type".into(), OCI_MANIFEST.into()));
    let (origin, handle) = server(vec![bad]);
    assert!(
        Registry::new(&origin, "tools/app", RegistryAuth::Anonymous, true)
            .unwrap()
            .pull(&reference)
            .is_err()
    );
    handle.join().unwrap();
}
#[test]
fn foreign_upload_location_and_redirects_are_not_followed() {
    for (status, location) in [
        (
            "202 Accepted",
            "http://attacker.invalid/v2/tools/app/blobs/uploads/stolen",
        ),
        (
            "307 Temporary Redirect",
            "http://attacker.invalid/credential-sink",
        ),
        ("202 Accepted", "/v2/foreign/repo/blobs/uploads/stolen"),
    ] {
        let mut response = reply(status, vec![]);
        response.headers.push(("Location".into(), location.into()));
        let (origin, handle) = server(vec![response]);
        assert!(
            Registry::new(&origin, "tools/app", RegistryAuth::Anonymous, true)
                .unwrap()
                .push("release", bundle())
                .is_err()
        );
        assert_eq!(handle.join().unwrap().len(), 1);
    }
}
#[test]
fn payload_size_digest_and_type_contracts_reject() {
    assert!(
        Descriptor::new(PAYLOAD_BLOB_TYPE, b"original")
            .validate(b"modified")
            .is_err()
    );
    let mut descriptor = Descriptor::new(PAYLOAD_BLOB_TYPE, b"original");
    descriptor.size += 1;
    assert!(descriptor.validate(b"original").is_err());
    let registry = Registry::new(
        "http://127.0.0.1:1/",
        "tools/app",
        RegistryAuth::Anonymous,
        true,
    )
    .unwrap();
    assert!(registry.pull("latest").is_err());
    let mut invalid = bundle();
    invalid.blobs.insert("not-a-digest".into(), vec![]);
    assert!(registry.push("release", invalid).is_err());
}

fn upload_replies(count: usize) -> Vec<Reply> {
    let mut replies = Vec::new();
    for _ in 0..count {
        let mut begin = reply("202 Accepted", vec![]);
        begin.headers.push((
            "Location".into(),
            "/v2/tools/app/blobs/uploads/session".into(),
        ));
        replies.push(begin);
        replies.push(reply("201 Created", vec![]));
    }
    replies.push(reply("201 Created", vec![]));
    replies
}

fn transport_reply(bytes: Vec<u8>) -> Reply {
    let mut response = reply("200 OK", bytes);
    response
        .headers
        .push(("Content-Type".into(), OCI_MANIFEST.into()));
    response
}

#[test]
fn independent_manifest_roundtrip_deduplicates_blobs_and_binds_envelopes() {
    let original = external_bundle();
    let expected_manifest = original.manifest.clone();
    let expected_external = original
        .external_manifests
        .values()
        .next()
        .unwrap()
        .manifest
        .clone();
    let (origin, handle) = server(upload_replies(6));
    let identity = Registry::new(&origin, "tools/app", RegistryAuth::Anonymous, true)
        .unwrap()
        .push("release", original)
        .unwrap();
    let requests = handle.join().unwrap();
    assert_eq!(requests.len(), 13);
    assert_eq!(
        requests.last().unwrap().0,
        "PUT /v2/tools/app/manifests/release HTTP/1.1"
    );
    assert!(
        requests[..12]
            .iter()
            .all(|(request, _)| request.contains("/blobs/uploads/"))
    );
    let encoded = &requests.last().unwrap().1;
    let transport: TransportManifest = serde_json::from_slice(encoded).unwrap();
    assert_eq!(transport.layers.len(), 5);
    assert_eq!(transport.layers[2].media_type, EXTERNAL_MANIFEST_TYPE);
    assert_eq!(transport.layers[3].media_type, EXTERNAL_ENVELOPE_TYPE);
    assert_eq!(
        transport.layers[2].annotations.as_ref().unwrap().manifest,
        transport.layers[2].digest
    );
    assert_eq!(
        transport.layers[3].annotations.as_ref().unwrap().manifest,
        transport.layers[2].digest
    );
    let uploaded: BTreeMap<_, _> = requests[..12]
        .iter()
        .filter(|(request, _)| request.starts_with("PUT "))
        .map(|(_, bytes)| (format!("sha256:{}", digest(bytes)), bytes.clone()))
        .collect();
    let mut replies = vec![transport_reply(encoded.clone())];
    replies.extend(
        std::iter::once(&transport.config)
            .chain(&transport.layers)
            .map(|descriptor| reply("200 OK", uploaded[&descriptor.digest].clone())),
    );
    let (origin, handle) = server(replies);
    let pulled = Registry::new(&origin, "tools/app", RegistryAuth::Anonymous, true)
        .unwrap()
        .pull(&identity)
        .unwrap();
    assert_eq!(pulled.manifest, expected_manifest);
    assert_eq!(pulled.envelope, b"root signature");
    assert_eq!(pulled.external_manifests.len(), 1);
    let external = &pulled.external_manifests[&digest(&expected_external)];
    assert_eq!(external.manifest, expected_external);
    assert_eq!(external.envelope, b"upstream signature");
    assert_eq!(pulled.blobs.len(), 1);
    assert_eq!(handle.join().unwrap().len(), 7);

    // Exact descriptor bytes are mandatory. Neither corrupt bytes nor a missing
    // independent object may cause fallback or recursive registry discovery.
    for missing in [false, true] {
        let mut replies = vec![transport_reply(encoded.clone())];
        replies.extend(
            std::iter::once(&transport.config)
                .chain(&transport.layers[..2])
                .map(|descriptor| reply("200 OK", uploaded[&descriptor.digest].clone())),
        );
        if missing {
            replies.push(reply("404 Not Found", vec![]));
        } else {
            let mut tampered = uploaded[&transport.layers[2].digest].clone();
            tampered[0] ^= 1;
            replies.push(reply("200 OK", tampered));
        }
        let (origin, handle) = server(replies);
        let result = Registry::new(&origin, "tools/app", RegistryAuth::Anonymous, true)
            .unwrap()
            .pull(&identity);
        assert!(result.is_err());
        let requests = handle.join().unwrap();
        assert_eq!(requests.len(), 5);
        assert!(
            requests
                .iter()
                .all(|(request, _)| !request.contains("referrers"))
        );
    }

    // A complete outer OCI object still cannot omit a referenced manifest.
    let mut omitted: TransportManifest = serde_json::from_slice(encoded).unwrap();
    omitted.layers.drain(2..4);
    let omitted_bytes = serde_json::to_vec(&omitted).unwrap();
    let omitted_identity = format!("sha256:{}", digest(&omitted_bytes));
    let mut replies = vec![transport_reply(omitted_bytes)];
    replies.extend(
        std::iter::once(&omitted.config)
            .chain(&omitted.layers)
            .map(|descriptor| reply("200 OK", uploaded[&descriptor.digest].clone())),
    );
    let (origin, handle) = server(replies);
    assert!(
        Registry::new(&origin, "tools/app", RegistryAuth::Anonymous, true)
            .unwrap()
            .pull(&omitted_identity)
            .is_err()
    );
    assert_eq!(handle.join().unwrap().len(), 5);
}

#[test]
fn external_manifest_bindings_and_total_transfer_budget_reject() {
    let fixture = external_bundle();
    let (external_id, external) = fixture.external_manifests.iter().next().unwrap();
    let base_layers = || {
        vec![
            Descriptor::new(PAYLOAD_TYPE, &fixture.manifest),
            Descriptor::new(ENVELOPE_TYPE, &fixture.envelope),
            Descriptor::external(EXTERNAL_MANIFEST_TYPE, &external.manifest, external_id),
            Descriptor::external(EXTERNAL_ENVELOPE_TYPE, &external.envelope, external_id),
        ]
    };
    let mut transport = TransportManifest {
        schema_version: 2,
        media_type: OCI_MANIFEST.into(),
        artifact_type: ARTIFACT_TYPE.into(),
        config: Descriptor::new(EMPTY_CONFIG_TYPE, b"{}"),
        layers: base_layers(),
    };
    transport.layers[3].annotations.as_mut().unwrap().manifest =
        format!("sha256:{}", "a".repeat(64));
    let bytes = serde_json::to_vec(&transport).unwrap();
    let identity = format!("sha256:{}", digest(&bytes));
    let (origin, handle) = server(vec![
        transport_reply(bytes),
        reply("200 OK", b"{}".to_vec()),
        reply("200 OK", fixture.manifest.clone()),
        reply("200 OK", fixture.envelope.clone()),
    ]);
    assert!(
        Registry::new(&origin, "tools/app", RegistryAuth::Anonymous, true)
            .unwrap()
            .pull(&identity)
            .is_err()
    );
    assert_eq!(handle.join().unwrap().len(), 4);

    transport.layers = base_layers();
    transport.layers[2].size = MAX_PAYLOAD_BYTES as u64;
    let bytes = serde_json::to_vec(&transport).unwrap();
    let identity = format!("sha256:{}", digest(&bytes));
    let (origin, handle) = server(vec![transport_reply(bytes)]);
    assert!(
        Registry::new(&origin, "tools/app", RegistryAuth::Anonymous, true)
            .unwrap()
            .pull(&identity)
            .is_err()
    );
    assert_eq!(handle.join().unwrap().len(), 1);
}
