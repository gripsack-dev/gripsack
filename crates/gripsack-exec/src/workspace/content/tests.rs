//! File preparation tests isolate captured-byte and logical-selection policy.
use super::*;
use gripsack_ir::Span;

fn span() -> Span {
    Span {
        file: "gripsack.ts".into(),
        line: 1,
        col: None,
    }
}

fn stage(capture: &Capture, name: &str) -> PathBuf {
    let path = capture.temporary.path().join(name);
    std::fs::create_dir(&path).unwrap();
    path
}

fn tree(
    capture: &mut Capture,
    include: &[&str],
    exclude: &[&str],
) -> Result<Vec<PreparedFile>, ExecError> {
    let destination = WorkspaceDestination::TrackedCopy {
        path: "~/.tree".into(),
    };
    let span = span();
    let declaration = File {
        span: &span,
        source: Some(Source::RepoTree {
            path: "tree".into(),
            include: include.iter().map(|value| (*value).into()).collect(),
            exclude: exclude.iter().map(|value| (*value).into()).collect(),
        }),
        content: Content::Identity,
        destination: &destination,
        checks: &[],
    };
    let stage = stage(capture, "tree-stage");
    let mut prepared = Vec::new();
    prepare_into(capture, &declaration, &stage, &mut prepared)?;
    Ok(prepared)
}

#[cfg(unix)]
#[test]
fn evaluated_file_aliases_use_captured_bytes_and_mode_for_identity_and_templates() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::sync::Arc;
    use store::source_bundle::{SourceBundle, SourceCapturePolicy};
    use store::trust::evaluation::EvaluationId;

    for absolute in [false, true] {
        let temporary = tempfile::tempdir().unwrap();
        let repo = temporary.path().join("repo");
        let frontend = temporary.path().join("frontend");
        let home = temporary.path().join("home");
        for directory in [&repo, &frontend, &home] {
            std::fs::create_dir(directory).unwrap();
        }
        std::fs::write(repo.join("gripsack.ts"), "// captured workspace\n").unwrap();
        std::fs::write(frontend.join("driver.ts"), "// captured frontend\n").unwrap();
        let target = repo.join("target");
        std::fs::write(&target, "value={{ value }}\n").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
        let alias = repo.join("alias");
        symlink(
            if absolute {
                target.as_path()
            } else {
                Path::new("target")
            },
            &alias,
        )
        .unwrap();
        let sources = SourceBundle::capture(
            &repo,
            &frontend,
            None,
            &home,
            SourceCapturePolicy::default(),
        )
        .unwrap();
        let repository = crate::Repository::evaluated(
            Arc::new(sources),
            EvaluationId::parse(&"a".repeat(64)).unwrap(),
        );
        let mut direct = Capture::new(&repo, Default::default()).unwrap();
        assert!(direct.file("alias").is_err());

        // Approval freezes both alias selection and the selected target.
        std::fs::remove_file(&alias).unwrap();
        std::fs::write(repo.join("replacement"), "live replacement\n").unwrap();
        symlink("replacement", &alias).unwrap();
        std::fs::write(&target, "live changed target\n").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).unwrap();

        let mut capture = Capture::from_repository(&repository, Default::default()).unwrap();
        let source = capture.file("alias").unwrap();
        assert_eq!(std::fs::read(&source.path).unwrap(), b"value={{ value }}\n");
        assert!(source.executable);
        let output = stage(&capture, "render");
        let span = span();
        let destination = WorkspaceDestination::TrackedCopy {
            path: "~/.script".into(),
        };
        let variables = BTreeMap::from([("value".into(), "approved".into())]);
        let file = File {
            span: &span,
            source: Some(Source::RepoFile {
                path: "alias".into(),
            }),
            content: Content::Identity,
            destination: &destination,
            checks: &[],
        };
        let identity = prepare_file(&mut capture, &file, &output).unwrap();
        assert_eq!(
            std::fs::read(output.join(&identity.entry.from)).unwrap(),
            b"value={{ value }}\n"
        );
        let template = File {
            content: Content::TemplateFile {
                variables: &variables,
                result_digest: None,
            },
            ..file
        };
        let rendered = prepare_file(&mut capture, &template, &output).unwrap();
        assert_eq!(
            std::fs::read(output.join(&rendered.entry.from)).unwrap(),
            b"value=approved\n"
        );
    }
}

#[test]
fn template_file_reads_captured_body_and_binds_source_variables_and_digest() {
    let repo = tempfile::tempdir().unwrap();
    std::fs::write(repo.path().join("script"), "value={{ value }}\n").unwrap();
    let mut capture = Capture::new(repo.path(), Default::default()).unwrap();
    let render_stage = stage(&capture, "render");
    let span = span();
    let destination = WorkspaceDestination::TrackedCopy {
        path: "~/.script".into(),
    };
    let variables = BTreeMap::from([("value".into(), "one".into())]);
    let file = File {
        span: &span,
        source: Some(Source::RepoFile {
            path: "script".into(),
        }),
        content: Content::TemplateFile {
            variables: &variables,
            result_digest: None,
        },
        destination: &destination,
        checks: &[],
    };
    let first = prepare_file(&mut capture, &file, &render_stage).unwrap();
    assert_eq!(
        std::fs::read(render_stage.join(&first.entry.from)).unwrap(),
        b"value=one\n"
    );
    std::fs::write(repo.path().join("script"), "changed={{ value }}\n").unwrap();
    // One preparation owns one captured observation, not a second live read.
    assert_eq!(
        prepare_file(&mut capture, &file, &render_stage)
            .unwrap()
            .entry
            .from,
        first.entry.from
    );
    let mut next = Capture::new(repo.path(), Default::default()).unwrap();
    let next_stage = stage(&next, "next");
    let changed = prepare_file(&mut next, &file, &next_stage).unwrap();
    assert_ne!(first.entry.from, changed.entry.from);
    let variables = BTreeMap::from([("value".into(), "two".into())]);
    let changed_vars = File {
        content: Content::TemplateFile {
            variables: &variables,
            result_digest: None,
        },
        ..file
    };
    let second = prepare_file(&mut next, &changed_vars, &next_stage).unwrap();
    assert_ne!(second.entry.from, changed.entry.from);
    let digest = store::hash::hex_sha256(b"changed=two\n");
    let digest_file = File {
        content: Content::TemplateFile {
            variables: &variables,
            result_digest: Some(&digest),
        },
        ..changed_vars
    };
    assert!(prepare_file(&mut next, &digest_file, &next_stage).is_ok());
    let wrong = "0".repeat(64);
    let invalid = File {
        content: Content::TemplateFile {
            variables: &variables,
            result_digest: Some(&wrong),
        },
        ..digest_file
    };
    assert!(prepare_file(&mut next, &invalid, &next_stage).is_err());
}

#[cfg(unix)]
#[test]
fn template_file_normalizes_executability_and_binds_mode_identity() {
    use std::os::unix::fs::PermissionsExt;
    let repo = tempfile::tempdir().unwrap();
    let source = repo.path().join("script");
    std::fs::write(&source, "#!/bin/sh\necho {{ value }}\n").unwrap();
    let span = span();
    let variables = BTreeMap::from([("value".into(), "ok".into())]);
    let destination = WorkspaceDestination::Symlink {
        path: "~/.script".into(),
    };
    let file = File {
        span: &span,
        source: Some(Source::RepoFile {
            path: "script".into(),
        }),
        content: Content::TemplateFile {
            variables: &variables,
            result_digest: None,
        },
        destination: &destination,
        checks: &[],
    };
    let mut identities = Vec::new();
    for (mode, expected) in [(0o644, 0o600), (0o755, 0o700), (0o711, 0o700)] {
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(mode)).unwrap();
        let mut capture = Capture::new(repo.path(), Default::default()).unwrap();
        let stage = stage(&capture, "mode");
        let prepared = prepare_file(&mut capture, &file, &stage).unwrap();
        assert_eq!(
            std::fs::metadata(stage.join(&prepared.entry.from))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            expected
        );
        identities.push(prepared.entry.from);
    }
    assert_ne!(identities[0], identities[1]);
    assert_eq!(identities[1], identities[2]);
}

#[cfg(unix)]
#[test]
fn repository_aliases_preserve_layout_and_filter_before_descent() {
    use std::os::unix::fs::symlink;
    let repo = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(repo.path().join("tree/real/deep")).unwrap();
    std::fs::write(repo.path().join("tree/real/deep/file"), "contents").unwrap();
    symlink("real", repo.path().join("tree/alias")).unwrap();
    symlink("real/deep/file", repo.path().join("tree/file-alias")).unwrap();
    symlink("/etc/passwd", repo.path().join("tree/hostile")).unwrap();
    symlink("absent", repo.path().join("tree/dangling")).unwrap();
    symlink(".", repo.path().join("tree/cycle")).unwrap();
    std::os::unix::net::UnixListener::bind(repo.path().join("tree/socket")).unwrap();
    let mut capture = Capture::new(repo.path(), Default::default()).unwrap();
    let files = tree(&mut capture, &["alias/deep/file", "file-alias"], &[]).unwrap();
    assert_eq!(
        files
            .iter()
            .map(|file| file.entry.to.as_str())
            .collect::<Vec<_>>(),
        ["~/.tree/alias/deep/file", "~/.tree/file-alias"]
    );
    let mut capture = Capture::new(repo.path(), Default::default()).unwrap();
    let files = tree(
        &mut capture,
        &[],
        &["hostile", "dangling", "cycle", "socket"],
    )
    .unwrap();
    assert_eq!(files.len(), 3);
    for file in files {
        assert_eq!(
            std::fs::read(
                capture
                    .temporary
                    .path()
                    .join("tree-stage")
                    .join(file.entry.from)
            )
            .unwrap(),
            b"contents"
        );
    }
}

#[cfg(unix)]
#[test]
fn repository_alias_cycles_escapes_dangling_and_sdk_roots_are_refused() {
    use std::os::unix::fs::symlink;
    for target in [".", "missing", "/etc/passwd", "../../sdk/file"] {
        let bundle = tempfile::tempdir().unwrap();
        let repo = bundle.path().join("repo");
        std::fs::create_dir_all(repo.join("tree")).unwrap();
        std::fs::create_dir(bundle.path().join("sdk")).unwrap();
        std::fs::write(bundle.path().join("sdk/file"), "not repository authority").unwrap();
        symlink(target, repo.join("tree/alias")).unwrap();
        let mut capture = Capture::new(&repo, Default::default()).unwrap();
        assert!(tree(&mut capture, &[], &[]).is_err(), "accepted {target}");
    }
}

#[test]
fn repository_budgets_count_unselected_entries_and_selected_bytes() {
    let repo = tempfile::tempdir().unwrap();
    std::fs::create_dir(repo.path().join("tree")).unwrap();
    std::fs::write(repo.path().join("tree/one"), "one").unwrap();
    std::fs::write(repo.path().join("tree/two"), "two").unwrap();
    for bound in ["entries", "metadata", "bytes", "file"] {
        let mut capture = Capture::new(repo.path(), Default::default()).unwrap();
        match bound {
            "entries" => capture.maximum_entries = 1,
            "metadata" => capture.maximum_metadata = 1,
            "bytes" => capture.maximum_tree_bytes = 2,
            "file" => capture.maximum_bytes = 2,
            _ => unreachable!(),
        }
        assert!(
            tree(&mut capture, &["one"], &[]).is_err(),
            "ignored {bound} budget"
        );
    }
}

#[cfg(unix)]
#[test]
fn alias_retarget_changes_identity_even_when_target_bytes_match() {
    use std::os::unix::fs::symlink;
    let repo = tempfile::tempdir().unwrap();
    std::fs::create_dir(repo.path().join("tree")).unwrap();
    for target in ["first", "second"] {
        std::fs::write(repo.path().join("tree").join(target), "same bytes").unwrap();
    }
    let alias = repo.path().join("tree/alias");
    symlink("first", &alias).unwrap();
    let mut capture = Capture::new(repo.path(), Default::default()).unwrap();
    let first = tree(&mut capture, &["alias"], &[]).unwrap();
    std::fs::remove_file(&alias).unwrap();
    symlink("second", &alias).unwrap();
    let mut capture = Capture::new(repo.path(), Default::default()).unwrap();
    let second = tree(&mut capture, &["alias"], &[]).unwrap();
    assert_eq!(first[0].entry.to, second[0].entry.to);
    assert_ne!(first[0].entry.from, second[0].entry.from);
}

#[test]
fn repository_logical_depth_is_bounded() {
    let repo = tempfile::tempdir().unwrap();
    let mut nested = repo.path().join("tree");
    for _ in 0..selection::MAX_TREE_DEPTH {
        nested.push("child");
    }
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(nested.join("file"), "bounded").unwrap();
    let mut capture = Capture::new(repo.path(), Default::default()).unwrap();
    let error = tree(&mut capture, &[], &[]).err().unwrap();
    assert!(error.to_string().contains("path-depth"));
}
