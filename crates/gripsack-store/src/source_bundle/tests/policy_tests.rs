use super::*;

fn policy(paths: &[&str]) -> SourceCapturePolicy {
    SourceCapturePolicy::new(paths.iter().map(|path| (*path).to_owned()).collect()).unwrap()
}

fn capture(fixture: &Fixture, policy: &SourceCapturePolicy) -> io::Result<SourceBundle> {
    SourceBundle::capture(
        &fixture.repo,
        &fixture.frontend,
        Some(&fixture.pin),
        &fixture.home,
        policy.clone(),
    )
}

#[test]
fn exclusions_are_bounded_literals_or_globs_and_protect_required_sources() {
    for path in [
        "",
        ".",
        "..",
        "/",
        "/tmp",
        "../secret",
        "a/../b",
        "a/./b",
        "a//b",
        "a/",
        "a\\b",
        "[a]",
        "a?b]",
        "a\0b",
        "a\nb",
        "C:/x",
        "C:/*",
        "env.toml",
        "gripsack.ts",
        "hosts",
        "hosts/selected.ts",
        "gripsack.lock",
        "locks",
        "locks/testhost.lock",
        "Gripsack.Lock",
        "LOCKS/testhost.lock",
        // Globs stay bounded: no whole-repository match, no `**` inside a
        // component, no character classes.
        "**",
        "**/**",
        "a**",
        "**b",
        "a**/b",
        "[ab]/*",
        // Required roots are unreachable by any root-aligned spelling.
        "hosts/**",
        "**/env.toml",
        "**/locks/**",
        "**/Gripsack.Lock",
    ] {
        assert!(
            SourceCapturePolicy::new(vec![path.to_owned()]).is_err(),
            "{path:?}"
        );
    }
    for paths in [
        vec!["a", "a"],
        vec!["a", "a-b", "a/child"],
        vec!["*.ts", "*.ts"],
    ] {
        assert!(SourceCapturePolicy::new(paths.into_iter().map(str::to_owned).collect()).is_err());
    }
    assert!(SourceCapturePolicy::new(vec!["a/".repeat(inventory::MAX_DEPTH) + "b"]).is_err());
    assert!(SourceCapturePolicy::new(vec!["*/".repeat(inventory::MAX_DEPTH) + "b"]).is_err());
    assert!(
        SourceCapturePolicy::new(vec!["a".repeat(inventory::MAX_INVENTORY_BYTES + 1)]).is_err()
    );
    let admitted = policy(&["a-b", "a/child", ".venv"]);
    assert!(admitted.excludes(Path::new(".venv/bin/python3")));
    assert!(!admitted.excludes(Path::new(".venv-local/bin/python3")));
    assert!(admitted.requires_directory(Path::new("a")));
    assert!(!admitted.requires_directory(Path::new("a-b")));
    // Glob spellings are recorded verbatim, sorted like literals.
    let globs = policy(&[".ruff_cache/**", "**/__pycache__", "build/*/target"]);
    assert_eq!(
        globs.exclusions(),
        ["**/__pycache__", ".ruff_cache/**", "build/*/target"]
    );
    assert!(globs.excludes(Path::new("pkg/sub/__pycache__")));
    assert!(globs.excludes(Path::new("__pycache__")));
    assert!(globs.excludes(Path::new(".ruff_cache")));
    assert!(globs.excludes(Path::new("build/anything/target")));
    assert!(!globs.excludes(Path::new("build/target")));
    assert!(!globs.excludes(Path::new("build/a/b/target")));
    assert!(!globs.excludes(Path::new("pkg/__pycache__x")));
    assert!(!globs.excludes(Path::new("__pycache__s/sub")));
    assert!(globs != policy(&["**/__pycache__"]));
}

#[test]
fn glob_exclusions_omit_nested_caches_exactly_like_literal_subtrees() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.repo.join("node_modules")).unwrap();
    fs::remove_file(fixture.repo.join("vendor_modules/@gripsack/core")).unwrap();
    fs::create_dir_all(fixture.repo.join("pkg/sub/__pycache__")).unwrap();
    fs::write(fixture.repo.join("pkg/sub/__pycache__/entry"), b"cache").unwrap();
    fs::create_dir_all(fixture.repo.join("tools/__pycache__")).unwrap();
    fs::create_dir(fixture.repo.join(".ruff_cache")).unwrap();
    fs::write(fixture.repo.join(".ruff_cache/entry.json"), b"cache").unwrap();
    // A suffix-anchored glob must not pin unrelated ancestor directories.
    fs::create_dir(fixture.repo.join("real-docs")).unwrap();
    symlink("real-docs", fixture.repo.join("docs")).unwrap();
    let excluded = policy(&["**/__pycache__", ".ruff_cache/**"]);
    assert_eq!(excluded.exclusions(), ["**/__pycache__", ".ruff_cache/**"]);
    let captured = SourceBundle::capture(
        &fixture.repo,
        &fixture.frontend,
        None,
        &fixture.home,
        excluded,
    )
    .unwrap();
    let exclusions = captured.inventory().exclusions();
    for path in [
        "repo/pkg/sub/__pycache__",
        "repo/tools/__pycache__",
        "repo/.ruff_cache",
    ] {
        assert!(exclusions.iter().any(|excluded| excluded == path), "{path}");
    }
    assert!(!captured.repository().join(".ruff_cache").exists());
    assert!(!captured.inventory().entries().iter().any(|entry| {
        entry.path.starts_with("repo/.ruff_cache")
            || entry.path.starts_with("repo/pkg/sub/__pycache__")
            || entry.path.starts_with("repo/tools/__pycache__")
    }));
    assert!(captured.inventory().entry("repo/docs").is_some());
    assert_eq!(
        captured
            .materialization_path(Path::new("pkg/sub/__pycache__/entry"))
            .unwrap_err()
            .kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_eq!(
        captured
            .materialization_path(Path::new("./.ruff_cache/entry.json"))
            .unwrap_err()
            .kind(),
        io::ErrorKind::PermissionDenied
    );
}

#[test]
fn glob_aliases_into_exclusions_and_replaced_ancestors_are_refused() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.repo.join("node_modules")).unwrap();
    fs::remove_file(fixture.repo.join("vendor_modules/@gripsack/core")).unwrap();
    fs::create_dir_all(fixture.repo.join("pkg/__pycache__")).unwrap();
    fs::write(fixture.repo.join("pkg/__pycache__/secret"), b"excluded").unwrap();
    symlink("pkg/__pycache__", fixture.repo.join("alternate")).unwrap();
    let error = SourceBundle::capture(
        &fixture.repo,
        &fixture.frontend,
        None,
        &fixture.home,
        policy(&["**/__pycache__"]),
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("excluded subtree repo/pkg/__pycache__"),
        "{error}"
    );

    // A non-suffix-anchored glob must pin its ancestors as real directories:
    // an alias there could re-expose excluded bytes under the target's
    // spelling, exactly as a replaced literal ancestor can.
    fs::remove_file(fixture.repo.join("alternate")).unwrap();
    fs::create_dir_all(fixture.repo.join("elsewhere/target")).unwrap();
    fs::write(fixture.repo.join("elsewhere/target/secret"), b"excluded").unwrap();
    symlink("elsewhere", fixture.repo.join("build")).unwrap();
    let error = SourceBundle::capture(
        &fixture.repo,
        &fixture.frontend,
        None,
        &fixture.home,
        policy(&["build/*/target"]),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("exclusion ancestor repo/build"), "{error}");
}

#[test]
fn glob_exclusions_cannot_bypass_sdk_root_rules_or_pins() {
    let fixture = Fixture::new();
    for spelling in ["**/core", "node_modules/**", "node_modules/@gripsack/cor?"] {
        let error = capture(&fixture, &policy(&[spelling]))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("excluded SDK location"),
            "{spelling}: {error}"
        );
    }
    // The linked node_modules parent cannot become an exclusion anchor.
    let error = capture(&fixture, &policy(&["node_modules/@gripsack/x*"]))
        .unwrap_err()
        .to_string();
    assert!(error.contains("real directory ancestors"), "{error}");
    // A glob that could carve the repo spelling of an admitted package root
    // is refused exactly like a literal exclusion there. The root must be a
    // real in-repo directory, as the auxiliary-root literal test sets up.
    let package = fixture.repo.join("embedded-sdk");
    fs::create_dir(&package).unwrap();
    fs::write(
        package.join("package.json"),
        br#"{"name":"@gripsack/core"}"#,
    )
    .unwrap();
    let error = SourceBundle::capture(
        &fixture.repo,
        &fixture.frontend,
        Some(&package),
        &fixture.home,
        policy(&["**/__pycache__"]),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("overlaps an admitted pin root"), "{error}");
}

#[test]
fn glob_matches_on_required_roots_refuse_instead_of_omitting_authority() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.repo.join("node_modules")).unwrap();
    fs::remove_file(fixture.repo.join("vendor_modules/@gripsack/core")).unwrap();
    fs::create_dir(fixture.repo.join("locks")).unwrap();
    fs::write(fixture.repo.join("locks/testhost.lock"), b"{}").unwrap();
    fs::create_dir(fixture.repo.join("hosts")).unwrap();
    fs::write(fixture.repo.join("hosts/selected.ts"), b"export {};\n").unwrap();
    for spelling in ["**/*.lock", "ho*"] {
        let error = SourceBundle::capture(
            &fixture.repo,
            &fixture.frontend,
            None,
            &fixture.home,
            policy(&[spelling]),
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("required configuration, lock authority or entrypoints"),
            "{spelling}: {error}"
        );
    }
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
    assert_eq!(
        fs::read(captured.repository().join("ignored.ts")).unwrap(),
        b"export const version = 1;\n"
    );
    assert!(
        captured
            .inventory()
            .exclusions()
            .iter()
            .any(|path| path == "repo/.venv")
    );
    assert!(!captured.repository().join(".venv").exists());
    assert!(
        !captured
            .inventory()
            .entries()
            .iter()
            .any(|entry| entry.path.starts_with("repo/.venv"))
    );
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
    for name in [
        "./.venv/secret.ts".to_owned(),
        fixture.repo.join(".venv/secret.ts").display().to_string(),
    ] {
        let mapped = captured.native_path(&name).unwrap();
        assert!(Path::new(mapped.as_ref()).starts_with(captured.repository()));
        assert!(fs::read(mapped.as_ref()).is_err());
    }
    assert!(
        captured
            .original_mode(Path::new(".venv/secret.ts"))
            .is_err()
    );
    assert!(
        captured
            .materialization_path(Path::new(".venv/secret.ts"))
            .is_err()
    );
}

#[test]
fn exclusions_do_not_reimport_sdk_roots_and_require_real_ancestors() {
    let fixture = Fixture::new();
    // Existing fixture uses a linked node_modules parent. It remains valid for
    // ordinary pins, but cannot give exclusions two different subtree meanings.
    let excluded = policy(&["node_modules/@gripsack/core"]);
    let error = SourceBundle::capture(
        &fixture.repo,
        &fixture.frontend,
        None,
        &fixture.home,
        excluded.clone(),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("real directory ancestors"), "{error}");
    assert!(
        capture(&fixture, &excluded)
            .unwrap_err()
            .to_string()
            .contains("excluded SDK location")
    );

    fs::remove_file(fixture.repo.join("node_modules")).unwrap();
    fs::remove_file(fixture.repo.join("vendor_modules/@gripsack/core")).unwrap();
    fs::create_dir_all(fixture.repo.join("node_modules/@gripsack")).unwrap();
    // An excluded editor link may dangle. Capturing it must not canonicalize,
    // open package.json, or promote the linked target into a root.
    symlink(
        "/missing/editor-sdk",
        fixture.repo.join("node_modules/@gripsack/core"),
    )
    .unwrap();
    let bundle = SourceBundle::capture(
        &fixture.repo,
        &fixture.frontend,
        None,
        &fixture.home,
        excluded.clone(),
    )
    .unwrap();
    assert!(bundle.pinned_frontend().is_none());
    assert!(
        !bundle
            .repository()
            .join("node_modules/@gripsack/core")
            .exists()
    );
    assert!(
        bundle
            .inventory()
            .exclusions()
            .iter()
            .any(|path| path == "repo/node_modules/@gripsack/core")
    );
}

#[test]
fn excluded_ancestor_cannot_be_read_as_an_auxiliary_root() {
    let fixture = Fixture::new();
    let local = fixture.repo.join("local-sdk");
    fs::create_dir(&local).unwrap();
    fs::write(local.join("package.json"), br#"{"name":"@gripsack/core"}"#).unwrap();
    for exclusions in [policy(&["local-sdk"]), policy(&["local-sdk/secret"])] {
        let error = SourceBundle::capture(
            &fixture.repo,
            &fixture.frontend,
            Some(&local),
            &fixture.home,
            exclusions.clone(),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("overlaps an admitted pin root"), "{error}");
    }
    let error = SourceBundle::capture(
        &fixture.repo,
        fixture._temporary.path(),
        None,
        &fixture.home,
        SourceCapturePolicy::default(),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("must not contain the repository"), "{error}");
}

#[test]
fn pin_appearance_without_root_admission_is_refused() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.repo.join("vendor_modules/@gripsack/core")).unwrap();
    let package = fixture.repo.join("vendor_modules/@gripsack/core");
    fs::create_dir(&package).unwrap();
    fs::write(
        package.join("package.json"),
        br#"{"name":"@gripsack/core"}"#,
    )
    .unwrap();
    let error = SourceBundle::capture(
        &fixture.repo,
        &fixture.frontend,
        None,
        &fixture.home,
        SourceCapturePolicy::default(),
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("without an admitted package root"),
        "{error}"
    );
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
    let admission = super::super::policy::CaptureAdmission {
        policy: &policy,
        runtime_home: &home,
    };
    let destination = gripsack_fs::open(&destination).unwrap();
    let error = super::super::capture::copy_roots(
        &roots,
        &destination,
        &admission,
        &mut CaptureBudget::default(),
    )
    .unwrap_err()
    .to_string();
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
    assert_eq!(
        fs::read(captured.repository().join("other-spelling/ordinary")).unwrap(),
        b"admitted"
    );
    for path in [
        "private-zone/secret",
        "./private-zone//secret",
        "other-spelling/secret",
    ] {
        assert_eq!(
            captured
                .materialization_path(Path::new(path))
                .unwrap_err()
                .kind(),
            io::ErrorKind::PermissionDenied
        );
        assert_eq!(
            captured
                .visit_materialized([path], |_, _, _| Ok(()))
                .unwrap_err()
                .kind(),
            io::ErrorKind::PermissionDenied
        );
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
        &declared,
        &fixture.frontend,
        Some(&fixture.pin),
        &fixture.home,
        policy(&[".venv"]),
    )
    .unwrap();
    for root in [
        fixture._temporary.path(),
        fixture.repo.as_path(),
        declared.as_path(),
        excluded.as_path(),
        alias.as_path(),
        fixture.frontend.as_path(),
        fixture.pin.as_path(),
    ] {
        let error = captured.admit_evaluator_root(root).unwrap_err();
        assert_eq!(
            error.kind(),
            io::ErrorKind::PermissionDenied,
            "{}",
            root.display()
        );
        assert!(error.to_string().contains("overlaps live source"));
    }
    let safe = fixture._temporary.path().join("runtime-only");
    fs::create_dir(&safe).unwrap();
    assert_eq!(
        captured.admit_evaluator_root(&safe).unwrap(),
        safe.canonicalize().unwrap()
    );
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
        &declared,
        &fixture.frontend,
        Some(&fixture.pin),
        &fixture.home,
        policy(&[".venv"]),
    )
    .unwrap();
    fs::write(fixture.repo.join("ignored.ts"), b"later live source").unwrap();
    for root in [
        fixture.repo.as_path(),
        declared.as_path(),
        captured.repository(),
    ] {
        let path = root.join("ignored.ts");
        let relative = captured.repository_relative(&path).unwrap();
        assert_eq!(relative, Path::new("ignored.ts"));
        let selected = captured.materialization_path(relative).unwrap();
        assert_eq!(fs::read(selected).unwrap(), b"export const version = 1;\n");
        let excluded = root.join(".venv/secret");
        let relative = captured.repository_relative(&excluded).unwrap();
        assert_eq!(
            captured.materialization_path(relative).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
    }
    assert!(
        captured
            .repository_relative(&fixture._temporary.path().join("external"))
            .is_none()
    );
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
        &declared,
        &fixture.frontend,
        Some(&fixture.pin),
        &fixture.home,
        policy(&[".venv"]),
    )
    .unwrap();
    let path = alias.join(".venv/secret");
    let relative = captured.repository_relative(&path).unwrap();
    assert_eq!(relative, Path::new(".venv/secret"));
    assert_eq!(
        captured.materialization_path(relative).unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    assert!(captured.admit_evaluator_root(&alias).is_err());
}
