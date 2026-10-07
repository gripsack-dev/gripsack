use super::*;

fn policy(paths: &[&str]) -> SourceCapturePolicy {
    SourceCapturePolicy::new(paths.iter().map(|path| (*path).to_owned()).collect()).unwrap()
}

fn capture(fixture: &Fixture, policy: &SourceCapturePolicy) -> io::Result<SourceBundle> {
    SourceBundle::capture(&fixture.repo, &fixture.frontend, Some(&fixture.pin), &fixture.home, policy.clone())
}

#[test]
fn exclusions_are_literal_bounded_subtrees_not_roots_globs_or_required_sources() {
    for path in ["", ".", "..", "/", "/tmp", "../secret", "a/../b", "a/./b", "a//b", "a/",
        "a\\b", "*.ts", "a?", "[a]", "a\0b", "a\nb", "C:/x", "env.toml", "gripsack.ts", "hosts", "hosts/selected.ts",
        "gripsack.lock", "locks", "locks/testhost.lock", "Gripsack.Lock", "LOCKS/testhost.lock"] {
        assert!(SourceCapturePolicy::new(vec![path.to_owned()]).is_err(), "{path:?}");
    }
    for paths in [vec!["a", "a"], vec!["a", "a-b", "a/child"]] {
        assert!(SourceCapturePolicy::new(paths.into_iter().map(str::to_owned).collect()).is_err());
    }
    assert!(SourceCapturePolicy::new(vec!["a/".repeat(inventory::MAX_DEPTH) + "b"]).is_err());
    assert!(SourceCapturePolicy::new(vec!["a".repeat(inventory::MAX_INVENTORY_BYTES + 1)]).is_err());
    let admitted = policy(&["a-b", "a/child", ".venv"]);
    assert!(admitted.excludes(Path::new(".venv/bin/python3")));
    assert!(!admitted.excludes(Path::new(".venv-local/bin/python3")));
    assert!(admitted.requires_directory(Path::new("a")));
    assert!(!admitted.requires_directory(Path::new("a-b")));
}

#[test]
fn ignored_outbound_alias_names_the_logical_path_and_explicit_exclusion_omits_it() {
    let fixture = Fixture::new();
    fs::write(fixture.repo.join(".gitignore"), b".venv/\nignored.ts\n").unwrap();
    fs::create_dir_all(fixture.repo.join(".venv/bin")).unwrap();
    symlink("/usr/bin/python3", fixture.repo.join(".venv/bin/python3")).unwrap();
    let error = fixture.capture().unwrap_err().to_string();
    assert!(error.contains("repo/.venv/bin/python3"), "{error}");
    assert!(error.contains("escapes its admitted roots"), "{error}");
    let captured = capture(&fixture, &policy(&[".venv"])).unwrap();
    assert_eq!(fs::read(captured.repository().join("ignored.ts")).unwrap(), b"export const version = 1;\n");
    assert!(captured.inventory().exclusions().iter().any(|path| path == "repo/.venv"));
    assert!(!captured.repository().join(".venv").exists());
    assert!(!captured.inventory().entries().iter().any(|entry| entry.path.starts_with("repo/.venv")));
}

#[test]
fn aliases_cannot_read_excluded_content_or_escape_through_it_then_return() {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.repo.join(".venv/nested")).unwrap();
    fs::write(fixture.repo.join(".venv/secret.ts"), b"secret bytes").unwrap();
    let admission = policy(&[".venv"]);
    for target in [
        fixture.repo.join(".venv/secret.ts"),
        PathBuf::from(".venv"),
        PathBuf::from(".venv/nested/../../ignored.ts"),
    ] {
        symlink(&target, fixture.repo.join("alternate")).unwrap();
        let error = capture(&fixture, &admission).unwrap_err().to_string();
        assert!(error.contains("repo/alternate"), "{error}");
        assert!(error.contains("excluded subtree repo/.venv"), "{error}");
        fs::remove_file(fixture.repo.join("alternate")).unwrap();
    }
    let captured = capture(&fixture, &admission).unwrap();
    for name in ["./.venv/secret.ts".to_owned(), fixture.repo.join(".venv/secret.ts").display().to_string()] {
        let mapped = captured.native_path(&name).unwrap();
        assert!(Path::new(mapped.as_ref()).starts_with(captured.repository()));
        assert!(fs::read(mapped.as_ref()).is_err());
    }
    assert!(captured.original_mode(Path::new(".venv/secret.ts")).is_err());
    assert!(captured.materialization_path(Path::new(".venv/secret.ts")).is_err());
}

#[test]
fn exclusions_do_not_reimport_sdk_roots_and_require_real_ancestors() {
    let fixture = Fixture::new();
    // Existing fixture uses a linked node_modules parent. It remains valid for
    // ordinary pins, but cannot give exclusions two different subtree meanings.
    let excluded = policy(&["node_modules/@gripsack/core"]);
    let error = SourceBundle::capture(&fixture.repo, &fixture.frontend, None, &fixture.home, excluded.clone())
        .unwrap_err().to_string();
    assert!(error.contains("real directory ancestors"), "{error}");
    assert!(capture(&fixture, &excluded).unwrap_err().to_string().contains("excluded SDK location"));

    fs::remove_file(fixture.repo.join("node_modules")).unwrap();
    fs::remove_file(fixture.repo.join("vendor_modules/@gripsack/core")).unwrap();
    fs::create_dir_all(fixture.repo.join("node_modules/@gripsack")).unwrap();
    // An excluded editor link may dangle. Capturing it must not canonicalize,
    // open package.json, or promote the linked target into a root.
    symlink("/missing/editor-sdk", fixture.repo.join("node_modules/@gripsack/core")).unwrap();
    let bundle = SourceBundle::capture(&fixture.repo, &fixture.frontend, None, &fixture.home, excluded.clone()).unwrap();
    assert!(bundle.pinned_frontend().is_none());
    assert!(!bundle.repository().join("node_modules/@gripsack/core").exists());
    assert!(bundle.inventory().exclusions().iter().any(|path| path == "repo/node_modules/@gripsack/core"));
}

#[test]
fn excluded_ancestor_cannot_be_read_as_an_auxiliary_root() {
    let fixture = Fixture::new();
    let local = fixture.repo.join("local-sdk");
    fs::create_dir(&local).unwrap();
    fs::write(local.join("package.json"), br#"{"name":"@gripsack/core"}"#).unwrap();
    for exclusions in [policy(&["local-sdk"]), policy(&["local-sdk/secret"])] {
        let error = SourceBundle::capture(&fixture.repo, &fixture.frontend, Some(&local), &fixture.home, exclusions.clone())
            .unwrap_err().to_string();
        assert!(error.contains("overlaps an admitted pin root"), "{error}");
    }
    let error = SourceBundle::capture(&fixture.repo, fixture._temporary.path(), None, &fixture.home, SourceCapturePolicy::default())
        .unwrap_err().to_string();
    assert!(error.contains("must not contain the repository"), "{error}");
}

#[test]
fn pin_appearance_without_root_admission_is_refused() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.repo.join("vendor_modules/@gripsack/core")).unwrap();
    let package = fixture.repo.join("vendor_modules/@gripsack/core");
    fs::create_dir(&package).unwrap();
    fs::write(package.join("package.json"), br#"{"name":"@gripsack/core"}"#).unwrap();
    let error = SourceBundle::capture(&fixture.repo, &fixture.frontend, None, &fixture.home, SourceCapturePolicy::default())
        .unwrap_err().to_string();
    assert!(error.contains("without an admitted package root"), "{error}");
}

#[test]
fn exclusion_ancestor_replacement_is_refused_during_the_copy_walk() {
    let temporary = tempfile::tempdir().unwrap();
    let repo = temporary.path().join("repo");
    let frontend = temporary.path().join("frontend");
    let destination = temporary.path().join("capture");
    for directory in [&repo, &frontend, &destination] {
        fs::create_dir(directory).unwrap();
    }
    fs::create_dir(repo.join("a")).unwrap();
    fs::write(repo.join("a/secret"), b"excluded").unwrap();
    let roots = vec![
        CaptureRoot::open(SourceRootKind::Repository, &repo).unwrap(),
        CaptureRoot::open(SourceRootKind::Frontend, &frontend).unwrap(),
    ];
    let policy = policy(&["a/secret"]);
    policy.admit_roots(&roots).unwrap();
    fs::rename(repo.join("a"), repo.join("alternate")).unwrap();
    symlink("alternate", repo.join("a")).unwrap();
    let home = temporary.path().join("runtime");
    let admission = super::super::policy::CaptureAdmission { policy: &policy, runtime_home: &home };
    let destination = gripsack_fs::open(&destination).unwrap();
    let error = super::super::capture::copy_roots(
        &roots, &destination, &admission, &mut CaptureBudget::default(),
    ).unwrap_err().to_string();
    assert!(error.contains("exclusion ancestor repo/a"), "{error}");
}

#[test]
fn excluded_children_under_included_alias_parents_are_not_artifact_fallbacks() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.repo.join("private-zone")).unwrap();
    fs::write(fixture.repo.join("private-zone/secret"), b"excluded").unwrap();
    fs::write(fixture.repo.join("private-zone/ordinary"), b"admitted").unwrap();
    symlink("private-zone", fixture.repo.join("other-spelling")).unwrap();
    let captured = capture(&fixture, &policy(&["private-zone/secret"])).unwrap();
    assert_eq!(fs::read(captured.repository().join("other-spelling/ordinary")).unwrap(), b"admitted");
    for path in ["private-zone/secret", "./private-zone//secret", "other-spelling/secret"] {
        assert_eq!(captured.materialization_path(Path::new(path)).unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(captured.visit_materialized([path], |_, _, _| Ok(())).unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    }
}

#[test]
fn evaluator_grants_refuse_source_ancestors_subtrees_and_aliases() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.repo.join("absolute.ts")).unwrap();
    let excluded = fixture.repo.join(".venv/bin");
    fs::create_dir_all(&excluded).unwrap();
    let declared = fixture._temporary.path().join("declared-repo");
    symlink(&fixture.repo, &declared).unwrap();
    let alias = fixture._temporary.path().join("runtime-path-alias");
    symlink(&excluded, &alias).unwrap();
    let captured = SourceBundle::capture(
        &declared, &fixture.frontend, Some(&fixture.pin), &fixture.home, policy(&[".venv"]),
    ).unwrap();
    for root in [
        fixture._temporary.path(), fixture.repo.as_path(), declared.as_path(),
        excluded.as_path(), alias.as_path(), fixture.frontend.as_path(), fixture.pin.as_path(),
    ] {
        let error = captured.admit_evaluator_root(root).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{}", root.display());
        assert!(error.to_string().contains("overlaps live source"));
    }
    let safe = fixture._temporary.path().join("runtime-only");
    fs::create_dir(&safe).unwrap();
    assert_eq!(captured.admit_evaluator_root(&safe).unwrap(), safe.canonicalize().unwrap());
}

#[test]
fn every_admitted_repository_spelling_selects_captured_source_and_exclusions() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.repo.join("absolute.ts")).unwrap();
    fs::create_dir(fixture.repo.join(".venv")).unwrap();
    fs::write(fixture.repo.join(".venv/secret"), b"excluded").unwrap();
    let declared = fixture._temporary.path().join("declared-repo");
    symlink(&fixture.repo, &declared).unwrap();
    let captured = SourceBundle::capture(
        &declared, &fixture.frontend, Some(&fixture.pin), &fixture.home, policy(&[".venv"]),
    ).unwrap();
    fs::write(fixture.repo.join("ignored.ts"), b"later live source").unwrap();
    for root in [fixture.repo.as_path(), declared.as_path(), captured.repository()] {
        let path = root.join("ignored.ts");
        let relative = captured.repository_relative(&path).unwrap();
        assert_eq!(relative, Path::new("ignored.ts"));
        let selected = captured.materialization_path(relative).unwrap();
        assert_eq!(fs::read(selected).unwrap(), b"export const version = 1;\n");
        let excluded = root.join(".venv/secret");
        let relative = captured.repository_relative(&excluded).unwrap();
        assert_eq!(captured.materialization_path(relative).unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    }
    assert!(captured.repository_relative(&fixture._temporary.path().join("external")).is_none());
}

#[test]
fn repository_alias_with_parent_components_keeps_its_resolved_parent_spelling() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.repo.join("absolute.ts")).unwrap();
    let parent = fixture._temporary.path().canonicalize().unwrap();
    let anchor = parent.join("anchor");
    fs::create_dir(&anchor).unwrap();
    let alias = parent.join("repo-alias");
    symlink(&fixture.repo, &alias).unwrap();
    let declared = anchor.join("../repo-alias");
    fs::create_dir(fixture.repo.join(".venv")).unwrap();
    fs::write(fixture.repo.join(".venv/secret"), b"excluded").unwrap();
    let captured = SourceBundle::capture(
        &declared, &fixture.frontend, Some(&fixture.pin), &fixture.home, policy(&[".venv"]),
    ).unwrap();
    let path = alias.join(".venv/secret");
    let relative = captured.repository_relative(&path).unwrap();
    assert_eq!(relative, Path::new(".venv/secret"));
    assert_eq!(captured.materialization_path(relative).unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    assert!(captured.admit_evaluator_root(&alias).is_err());
}
