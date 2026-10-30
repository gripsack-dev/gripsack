use super::*;

fn json(before: &str, after: &str) -> ConfigView {
    compare_config(
        before.as_bytes(),
        after.as_bytes(),
        ConfigFormat::Json,
        ConfigScope::WholeFile,
        &ConfigViewOptions::default(),
    )
}

fn toml(before: &str, after: &str) -> ConfigView {
    compare_config(
        before.as_bytes(),
        after.as_bytes(),
        ConfigFormat::Toml,
        ConfigScope::WholeFile,
        &ConfigViewOptions::default(),
    )
}

#[test]
fn object_order_and_whitespace_change_bytes_not_structure() {
    let view = json(r#"{"b":2,"a":1}"#, "{\n  \"a\": 1, \"b\": 2\n}");
    assert_eq!(view.exact_bytes_equal, Some(false));
    assert_eq!(view.structural_equal, Some(true));
    assert!(view.complete);
    assert!(view.changes.is_empty());
    assert!(view.fallback.is_none());
}

#[test]
fn arrays_remain_ordered_and_changes_have_real_value_spans() {
    let before = r#"{"a":[1,2],"remove":true,"type":0}"#;
    let after = r#"{"a":[2,1],"add":false,"type":"0"}"#;
    let view = json(before, after);
    assert_eq!(view.structural_equal, Some(false));
    assert_eq!(view.changes.len(), 5);
    let change = view
        .changes
        .iter()
        .find(|change| change.path == "$[\"a\"][0]")
        .unwrap();
    assert_eq!(change.reason, ChangeReason::ValueChanged);
    let old = change.before_span.unwrap();
    let new = change.after_span.unwrap();
    assert_eq!(&before[old.start..old.end], "1");
    assert_eq!(&after[new.start..new.end], "2");
    assert!(
        view.changes
            .iter()
            .any(|change| change.reason == ChangeReason::Added && change.before_span.is_none())
    );
    assert!(
        view.changes
            .iter()
            .any(|change| change.reason == ChangeReason::Removed && change.after_span.is_none())
    );
    assert!(
        view.changes
            .iter()
            .any(|change| change.reason == ChangeReason::TypeChanged)
    );
}

#[test]
fn paths_distinguish_literal_dots_arrays_quotes_and_unicode_controls() {
    let before = serde_json::json!({"a.b": 1, "a": {"b": 1}, "a/\"~[0]": 1, "\u{202e}\u{0085}": 1});
    let after = serde_json::json!({"a.b": 2, "a": {"b": 2}, "a/\"~[0]": 2, "\u{202e}\u{0085}": 2});
    let view = json(&before.to_string(), &after.to_string());
    let paths: Vec<_> = view
        .changes
        .iter()
        .map(|change| change.path.as_str())
        .collect();
    assert!(paths.contains(&"$[\"a.b\"]"));
    assert!(paths.contains(&"$[\"a\"][\"b\"]"));
    assert!(paths.contains(&"$[\"a/\\\"~[0]\"]"));
    assert!(paths.contains(&"$[\"\\u202e\\u0085\"]"));
    assert!(
        paths
            .iter()
            .all(|path| !path.chars().any(|c| c.is_control() || format_control(c)))
    );
}

#[test]
fn decoded_duplicate_keys_at_any_depth_are_explicit_fallbacks() {
    for invalid in [
        r#"{"x":1,"x":2}"#,
        r#"{"x":1,"\u0078":2}"#,
        r#"{"outer":[{"x":1,"x":2}]}"#,
    ] {
        let view = json(invalid, "{}");
        assert_eq!(view.structural_equal, None);
        assert!(!view.complete);
        assert_eq!(
            view.fallback.unwrap().before_reason,
            Some(FallbackReason::DuplicateKey)
        );
    }
}

#[test]
fn invalid_json_never_becomes_empty_equal_structure() {
    for invalid in [
        "",
        "[1,]",
        "{\"a\":1,}",
        "{\"a\":01}",
        "{\"a\":NaN}",
        "true false",
        "\"\\ud800\"",
        "1e9999",
    ] {
        let view = json(invalid, invalid);
        assert_eq!(view.exact_bytes_equal, Some(true));
        assert_eq!(view.structural_equal, None);
        assert_eq!(
            view.fallback.unwrap().before_reason,
            Some(FallbackReason::InvalidSyntax)
        );
        assert!(
            view.unknown_reasons
                .contains(&UnknownReason::StructuralViewUnavailable)
        );
    }
}

#[test]
fn json_number_parser_distinctions_are_not_application_equivalence() {
    assert_eq!(json("1", "1.0").structural_equal, Some(false));
    assert_eq!(json("1e0", "1.0").structural_equal, Some(true));
}

#[test]
fn toml_comments_key_order_and_integer_radix_are_semantically_equal() {
    let view = toml(
        "a = 0x10\nb = [1, 2]\n",
        "# new comment\nb=[1,2]\na=16 # decimal\n",
    );
    assert_eq!(view.exact_bytes_equal, Some(false));
    assert_eq!(view.structural_equal, Some(true));
    assert!(view.changes.is_empty());
}

#[test]
fn toml_nested_tables_arrays_datetime_and_spans_follow_parser_types() {
    let before = "[service]\nport = 80\nwhen = 2026-10-28\nvalues = [1, 2]\n";
    let after = "[service]\nport = 80.0\nwhen = '2026-10-28'\nvalues = [2, 1]\n";
    let view = toml(before, after);
    assert!(view.fallback.is_none());
    assert_eq!(view.changes.len(), 4);
    let port = view
        .changes
        .iter()
        .find(|change| change.path == "$[\"service\"][\"port\"]")
        .unwrap();
    assert_eq!(port.reason, ChangeReason::TypeChanged);
    assert_eq!(port.before.as_ref().unwrap().kind, ValueKind::Integer);
    assert_eq!(port.after.as_ref().unwrap().kind, ValueKind::Float);
    let span = port.after_span.unwrap();
    assert_eq!(&after[span.start..span.end], "80.0");
    let when = view
        .changes
        .iter()
        .find(|change| change.path.ends_with("[\"when\"]"))
        .unwrap();
    assert_eq!(when.before.as_ref().unwrap().kind, ValueKind::Datetime);
    assert_eq!(when.after.as_ref().unwrap().kind, ValueKind::String);
}

#[test]
fn toml_dotted_and_quoted_keys_and_array_tables_have_selectable_spans() {
    let before = "'a.b' = 1\na.b = 2\n[[jobs]]\nname = 'old'\n";
    let after = "'a.b' = 3\na.b = 4\n[[jobs]]\nname = 'new'\n";
    let view = toml(before, after);
    assert_eq!(view.changes.len(), 3);
    let name = view
        .changes
        .iter()
        .find(|change| change.path == "$[\"jobs\"][0][\"name\"]")
        .unwrap();
    let span = name.after_span.unwrap();
    assert_eq!(&after[span.start..span.end], "'new'");
}

#[test]
fn toml_duplicate_invalid_and_numeric_overflow_are_explicit() {
    for invalid in [
        "a=1\na=2\n",
        "a=[1,",
        "a=9223372036854775808",
        "a=1e9999",
        "a = 2026-13-99",
    ] {
        let view = toml(invalid, invalid);
        assert_eq!(view.structural_equal, None);
        assert_eq!(
            view.fallback.unwrap().before_reason,
            Some(FallbackReason::InvalidSyntax)
        );
    }
    assert_eq!(toml("a=-0.0", "a=0.0").structural_equal, Some(true));
    // The stated contract is f64 PartialEq, not a fabricated NaN equivalence.
    assert_eq!(toml("a=nan", "a=nan").structural_equal, Some(false));
}

#[test]
fn hostile_depth_key_size_node_count_and_dotted_toml_are_bounded() {
    let nested = format!(
        "{}0{}",
        "[".repeat(MAX_DEPTH + 1),
        "]".repeat(MAX_DEPTH + 1)
    );
    let huge_key = format!("{{\"{}\":0}}", "k".repeat(MAX_PATH_BYTES + 1));
    let many_nodes = format!("[{}]", vec!["0"; MAX_NODES + 1].join(","));
    for input in [&nested, &huge_key, &many_nodes] {
        assert_eq!(
            json(input, "{}").fallback.unwrap().before_reason,
            Some(FallbackReason::ResourceLimit)
        );
    }
    let dotted = format!("{}leaf=0", "a.".repeat(MAX_DEPTH + 1));
    assert_eq!(
        toml(&dotted, "").fallback.unwrap().before_reason,
        Some(FallbackReason::ResourceLimit)
    );
    let quoted = format!("value = '{}'", ".".repeat(MAX_DEPTH * 2));
    assert!(toml(&quoted, &quoted).fallback.is_none());
}

#[test]
fn fallback_preserves_byte_difference_without_disclosing_unparsed_secrets() {
    let before = b"password: old-secret\n";
    let after = b"password: new-secret\n";
    let view = compare_config(
        before,
        after,
        ConfigFormat::Unsupported,
        ConfigScope::ForeignObservation,
        &ConfigViewOptions::local(),
    );
    assert_eq!(view.scope, ConfigScope::ForeignObservation);
    assert_eq!(view.exact_bytes_equal, Some(false));
    let fallback = view.fallback.as_ref().unwrap();
    assert_eq!(
        fallback.before_reason,
        Some(FallbackReason::UnsupportedFormat)
    );
    assert!(!fallback.differences.is_empty());
    assert!(!fallback.truncated);
    assert!(fallback.raw_bytes_omitted);
    let serialized = serde_json::to_string(&view).unwrap();
    assert!(!serialized.contains("old-secret"));
    assert!(!serialized.contains("new-secret"));
}

#[test]
fn binary_and_oversized_inputs_never_claim_structural_equality() {
    let binary = compare_config(
        b"\xff\0",
        b"\xfe\0",
        ConfigFormat::Json,
        ConfigScope::ManagedBlock,
        &ConfigViewOptions::default(),
    );
    assert_eq!(binary.scope, ConfigScope::ManagedBlock);
    assert!(binary.fallback.unwrap().binary_omitted);
    let mut old = vec![b' '; MAX_INPUT_BYTES + 1];
    let mut new = old.clone();
    new[MAX_INPUT_BYTES] = b'!';
    let oversized = compare_config(
        &old,
        &new,
        ConfigFormat::Json,
        ConfigScope::WholeFile,
        &ConfigViewOptions::default(),
    );
    assert_eq!(oversized.exact_bytes_equal, None);
    assert_eq!(oversized.structural_equal, None);
    assert!(oversized.truncated);
    assert_eq!(
        oversized.fallback.unwrap().before_reason,
        Some(FallbackReason::Oversized)
    );
    old[0] = b'x';
    assert_eq!(
        compare_config(
            &old,
            &new,
            ConfigFormat::Json,
            ConfigScope::WholeFile,
            &ConfigViewOptions::default()
        )
        .exact_bytes_equal,
        Some(false)
    );
}

#[test]
fn portable_reports_omit_all_raw_values_including_generic_fields() {
    let view = json(
        r#"{"ordinary":"unmarked-secret-old"}"#,
        r#"{"ordinary":"unmarked-secret-new"}"#,
    );
    let encoded = serde_json::to_string(&view).unwrap();
    assert!(!encoded.contains("unmarked-secret"));
    assert!(view.redacted);
    assert_eq!(
        view.changes[0].after.as_ref().unwrap().disclosure,
        ValueDisclosure::OmittedPortable
    );
}

#[test]
fn local_disclosure_redacts_secrets_ancestors_urls_keys_and_explicit_paths() {
    let after = serde_json::json!({"api_token":"token-value", "secret":{"nested":123}, "other":"Bearer private", "url":"https://user:credential@example.com", "custom":"custom-private", "https://user:key-secret@example.com": "x"});
    let mut options = ConfigViewOptions::local();
    options.secret_paths.push("$[\"custom\"]".to_owned());
    let view = compare_config(
        b"{}",
        after.to_string().as_bytes(),
        ConfigFormat::Json,
        ConfigScope::WholeFile,
        &options,
    );
    let encoded = serde_json::to_string(&view).unwrap();
    for forbidden in [
        "token-value",
        "Bearer private",
        "credential@example",
        "custom-private",
        "key-secret",
    ] {
        assert!(!encoded.contains(forbidden));
    }
    assert!(view.redacted);
    assert!(view.changes.iter().any(|change| change.path_redacted));
    let nested = compare_config(
        br#"{"secret":{"nested":123}}"#,
        br#"{"secret":{"nested":456}}"#,
        ConfigFormat::Json,
        ConfigScope::WholeFile,
        &options,
    );
    assert_eq!(
        nested.changes[0].after.as_ref().unwrap().disclosure,
        ValueDisclosure::Redacted
    );
}

#[test]
fn local_values_are_bounded_and_terminal_controls_cannot_escape() {
    let after =
        serde_json::json!({"name":"hi\u{001b}[31m\n\u{202e}", "long":"é".repeat(MAX_VALUE_BYTES)});
    let view = compare_config(
        b"{}",
        after.to_string().as_bytes(),
        ConfigFormat::Json,
        ConfigScope::WholeFile,
        &ConfigViewOptions::local(),
    );
    let name = view
        .changes
        .iter()
        .find(|change| change.path == "$[\"name\"]")
        .unwrap()
        .after
        .as_ref()
        .unwrap();
    assert!(name.text.as_ref().unwrap().contains("\\u{1b}"));
    assert!(
        !name
            .text
            .as_ref()
            .unwrap()
            .chars()
            .any(|c| c.is_control() || format_control(c))
    );
    let long = view
        .changes
        .iter()
        .find(|change| change.path == "$[\"long\"]")
        .unwrap()
        .after
        .as_ref()
        .unwrap();
    assert!(long.truncated);
    assert!(long.text.as_ref().unwrap().len() <= MAX_VALUE_BYTES);
    assert!(view.complete);
    assert!(view.truncated);
    assert!(
        view.unknown_reasons
            .contains(&UnknownReason::ValuesTruncated)
    );
}

#[test]
fn bounded_change_lists_keep_known_inequality_and_explicit_incompleteness() {
    let after: serde_json::Map<String, serde_json::Value> = (0..MAX_CHANGES + 1)
        .map(|index| (format!("key{index:04}"), serde_json::Value::Bool(true)))
        .collect();
    let view = json("{}", &serde_json::Value::Object(after).to_string());
    assert_eq!(view.structural_equal, Some(false));
    assert_eq!(view.changes.len(), MAX_CHANGES);
    assert!(!view.complete);
    assert!(view.truncated);
    assert!(
        view.unknown_reasons
            .contains(&UnknownReason::ChangesTruncated)
    );
}
