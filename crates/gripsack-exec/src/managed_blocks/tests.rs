use super::*;

fn block(module: &str, payload: &str, mode: u32) -> String {
    ManagedBlockSet::parse("", module)
        .unwrap()
        .upsert(module, Path::new("rc"), None, payload, mode)
        .unwrap()
}

#[test]
fn foreign_bytes_survive_reconcile_and_removal() {
    for newline in ["\n", "\r\n"] {
        let first = block("shell", "one", 0o600).replace('\n', newline);
        let second = block("shell", "edited", 0o600).replace('\n', newline);
        let before = format!(
            "foreign-prefix{newline}{first}foreign-between{newline}{second}foreign-tail{newline}{newline}"
        );
        let parsed = ManagedBlockSet::parse(&before, "shell").unwrap();
        let output = parsed
            .upsert("shell", Path::new("rc"), None, "desired", 0o600)
            .unwrap();
        let again = ManagedBlockSet::parse(&output, "shell").unwrap();
        assert_eq!(again.blocks().len(), 1);
        assert!(again.satisfied(&content_hash("desired"), 0o600));
        assert_eq!(
            again.remove().unwrap(),
            format!(
                "foreign-prefix{newline}foreign-between{newline}foreign-tail{newline}{newline}"
            )
        );
        assert_eq!(
            again
                .upsert("shell", Path::new("rc"), None, "desired", 0o600)
                .unwrap(),
            output
        );
    }
}

#[test]
fn malformed_markers_never_grant_splice_ranges() {
    let first = block("shell", "one", 0o644);
    let opener = first.lines().next().unwrap();
    for malformed in [
        format!("{first}{opener}\nUSER-TAIL\n"),
        format!("{opener}\n{first}USER-TAIL\n"),
        "# <<< gripsack module=shell <<<\nUSER-TAIL\n".into(),
        format!("{opener}\n# <<< gripsack module=other <<<\nUSER-TAIL\n"),
        "# >>> gripsack module=shell sha=not-a-hash >>>\nUSER-TAIL\n".into(),
    ] {
        assert!(
            ManagedBlockSet::parse(&malformed, "shell").is_err(),
            "{malformed}"
        );
    }
}

#[test]
fn every_mode_record_participates_independently_of_order() {
    for live in [0o600, 0o644, 0o755] {
        for left in [None, Some(0o600), Some(0o644), Some(0o755)] {
            for right in [None, Some(0o600), Some(0o644), Some(0o755)] {
                let render = |mode| match mode {
                    Some(mode) => block("shell", "one", mode),
                    None => block("shell", "one", 0o644).replace(" mode=0644", ""),
                };
                let text = render(left) + &render(right);
                let expected = [left, right].into_iter().flatten().any(|mode| mode != live);
                let parsed = ManagedBlockSet::parse(&text, "shell").unwrap();
                assert_eq!(parsed.mode_conflicts(live, None), expected);
                assert!(!parsed.satisfied(&content_hash("one"), live));
            }
        }
    }
}

#[test]
fn later_hand_edits_are_classified_and_other_modules_stay_untouched() {
    let first = block("shell", "one", 0o644);
    let edited = first.replace("one", "two");
    let other = block("other", "other-content", 0o644);
    let text = format!("{first}{other}{edited}");
    let parsed = ManagedBlockSet::parse(&text, "shell").unwrap();
    assert!(!parsed.blocks()[0].edited());
    assert!(parsed.blocks()[1].edited());
    let output = parsed
        .upsert("shell", Path::new("rc"), None, "desired", 0o644)
        .unwrap();
    assert_eq!(
        ManagedBlockSet::parse(&output, "shell")
            .unwrap()
            .remove()
            .unwrap(),
        other
    );
}

#[test]
fn markers_roundtrip_across_comment_styles_and_legacy_metadata() {
    for (dest, marker, prefix) in [
        ("rc", None, "#"),
        ("x.jsonc", None, "//"),
        (".vimrc", None, "\""),
        ("x.html", None, "<!--"),
        ("x.lua", None, "--"),
        ("rc", Some("#!"), "#!"),
    ] {
        let payload = "echo 'docs say <<< gripsack <<< ends a block'\n";
        let output = ManagedBlockSet::parse("user\n", "shell")
            .unwrap()
            .upsert("shell", Path::new(dest), marker, payload, 0o755)
            .unwrap();
        assert!(output.contains(&format!("{prefix} >>> gripsack module=shell")));
        let parsed = ManagedBlockSet::parse(&output, "shell").unwrap();
        assert!(parsed.satisfied(&content_hash(payload), 0o755));
        assert_eq!(parsed.remove().unwrap(), "user\n");
        let legacy = output.replace(" mode=0755", "");
        let legacy = ManagedBlockSet::parse(&legacy, "shell").unwrap();
        assert!(!legacy.satisfied(&content_hash(payload), 0o755));
        assert!(!legacy.mode_conflicts(0o755, None));
    }
}
