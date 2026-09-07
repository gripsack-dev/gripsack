use super::*;
use std::time::Instant;

// Execute source through sh, avoiding executable-file ETXTBSY races.
fn peer(body: &str) -> Command {
    let mut command = Command::new("/bin/sh");
    command.args(["-c", body]);
    command
}

fn exchange(
    body: &str,
    request: serde_json::Value,
    limits: FetchLimits,
) -> Result<PluginFetch, FetchError> {
    let dest = tempfile::tempdir().unwrap();
    std::fs::write(dest.path().join("payload"), b"hello").unwrap();
    fetch_exchange(
        &mut peer(body),
        "test",
        &request,
        dest.path(),
        Duration::from_secs(3),
        limits,
    )
}

#[test]
fn round_trip_and_reported_hash_verification() {
    let dest = tempfile::tempdir().unwrap();
    std::fs::write(dest.path().join("payload"), b"hello").unwrap();
    let expected = gripsack_store::canonical_tree_hash(dest.path()).unwrap();
    let response = serde_json::json!({"type":"response","result":{
        "url":"https://example/tarball", "version":"1.2.3", "sha256":expected.as_str()
    }});
    let body = format!("read line\nprintf '%s\\n' '{response}'");
    let got = fetch_exchange(
        &mut peer(&body),
        "test",
        &serde_json::json!({}),
        dest.path(),
        Duration::from_secs(3),
        FetchLimits::default(),
    )
    .unwrap();
    assert_eq!(got.tree.as_str(), expected.as_str());
    assert_eq!(got.url.as_deref(), Some("https://example/tarball"));
    assert_eq!(got.version.as_deref(), Some("1.2.3"));
    let err = exchange(
        "read line; printf '%s\\n' '{\"type\":\"response\",\"result\":{\"sha256\":\"wrong\"}}'",
        serde_json::json!({}),
        FetchLimits::default(),
    )
    .unwrap_err();
    assert!(err.to_string().contains("disagrees"));
}

#[test]
fn bounded_tree_is_checked_before_hash() {
    let limits = FetchLimits {
        expanded_bytes: std::num::NonZeroU64::new(4).unwrap(),
        ..FetchLimits::default()
    };
    let err = exchange(
        "read line; echo '{\"type\":\"response\"}'",
        serde_json::json!({}),
        limits,
    )
    .unwrap_err();
    assert!(matches!(err, FetchError::PayloadTooLarge { .. }));
}

#[test]
fn silent_and_response_then_linger_both_fail() {
    for body in [
        "read line; exec sleep 60",
        "read line; echo '{\"type\":\"response\"}'; exec sleep 60",
    ] {
        let start = Instant::now();
        let err = exchange(body, serde_json::json!({}), FetchLimits::default()).unwrap_err();
        assert!(err.to_string().contains("exchange deadline"), "{err}");
        assert!(start.elapsed() < Duration::from_secs(15));
    }
}

#[test]
fn response_does_not_hide_output_failure() {
    let err = exchange(
        "read line; echo '{\"type\":\"response\"}'; head -c 2097152 /dev/zero",
        serde_json::json!({}),
        FetchLimits::default(),
    )
    .unwrap_err();
    assert!(err.to_string().contains("1 MiB cap"), "{err}");
}

#[test]
fn simultaneous_pipe_pressure_and_early_response() {
    // Peer produces more than a pipeful before consuming a large request.
    // The response also arrives before stdin is consumed completely.
    let got = exchange(
        "head -c 262144 /dev/zero >&2; echo noise; echo '{\"type\":\"response\"}'; cat >/dev/null",
        serde_json::json!({"args":"x".repeat(512 * 1024)}),
        FetchLimits::default(),
    )
    .unwrap();
    assert!(got.url.is_none());
}

#[test]
fn no_response_or_nonzero_exit_is_not_a_fetch() {
    for body in [
        "cat >/dev/null",
        "read line; echo '{\"type\":\"response\"}'; exit 1",
    ] {
        assert!(exchange(body, serde_json::json!({}), FetchLimits::default()).is_err());
    }
}

#[test]
fn capability_results_require_a_clean_exit() {
    let response = "echo '{\"type\":\"response\",\"result\":{\"capabilities\":{\"throttle\":{\"example.com\":\"10/min\"}}}}'";
    let good = format!("read line; echo noise; {response}");
    let caps = capabilities::exchange(&mut peer(&good), Duration::from_secs(3)).unwrap();
    assert_eq!(
        caps.throttle.get("example.com").map(String::as_str),
        Some("10/min")
    );
    for body in [
        "read line; exec sleep 60".to_owned(),
        "cat >/dev/null".to_owned(),
        format!("read line; {response}; exec sleep 60"),
        format!("read line; {response}; exit 1"),
        format!("read line; {response}; head -c 2097152 /dev/zero"),
    ] {
        assert!(capabilities::exchange(&mut peer(&body), Duration::from_secs(3)).is_none());
    }
}
