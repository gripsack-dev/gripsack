use super::*;

// Actual upstream records, retained from prefix-dev/pixi revision
// 6f71156de33ac6420522a77f21437ff0a331cb09, tests/data/satisfiability/
// old_lock_file/pixi.lock. No solver or network is involved in these tests.
const CERT: &str =
    "https://conda.anaconda.org/conda-forge/linux-64/ca-certificates-2024.2.2-hbcca054_0.conda";
const CERT_SHA: &str = "91d81bfecdbb142c15066df70cc952590ae8991670198f92c66b62019b251aeb";
const TZ: &str = "https://conda.anaconda.org/conda-forge/noarch/tzdata-2024a-h0c530f3_0.conda";
const CERT_RECORD: &str = "  sha256: 91d81bfecdbb142c15066df70cc952590ae8991670198f92c66b62019b251aeb\n  md5: 2f4327a1cbe7f022401b236e915a5fef\n  license: ISC\n  purls: []\n  size: 155432\n  timestamp: 1706843687645\n";
const TZ_RECORD: &str = "  sha256: 7b2b69c54ec62a243eb6fba2391b5e443421608c3ae5dbff938ad33ca8db5122\n  md5: 161081fc7cec0bfda0d86d7cb595f8d8\n  license: LicenseRef-Public-Domain\n  purls: []\n  size: 119815\n  timestamp: 1706886945727\n";
const MANIFEST: &str = "[workspace]\nname = 'captured'\nchannels = ['conda-forge']\nplatforms = ['linux-64']\n[dependencies]\nca-certificates = { version = '>=2024.2,<2025', build = 'hbcca054_*' }\n";

fn lock(extra: bool) -> String {
    let mut lock = format!(
        "version: 6\nenvironments:\n  default:\n    channels:\n    - url: https://conda.anaconda.org/conda-forge/\n    packages:\n      linux-64:\n      - conda: {CERT}\n"
    );
    if extra {
        lock.push_str(&format!("      - conda: {TZ}\n"));
    }
    lock.push_str(&format!("packages:\n- conda: {CERT}\n{CERT_RECORD}"));
    if extra {
        lock.push_str(&format!("- conda: {TZ}\n{TZ_RECORD}"));
    }
    lock
}
fn request(manifest: &str, lock: String) -> ImportPixiRequest {
    ImportPixiRequest {
        attempt: 1,
        manifest_bytes: manifest.into(),
        lock_bytes: lock,
        environment: "default".into(),
        platform: "linux-64".into(),
    }
}
fn import(request: ImportPixiRequest) -> LockedCondaEnvironment {
    import_pixi(request).unwrap_or_else(|failure| panic!("{}: {}", failure.code, failure.message))
}
fn refuses(request: ImportPixiRequest, code: &str, name: &str) {
    match import_pixi(request) {
        Ok(_) => panic!("accepted invalid frozen import; expected {code}"),
        Err(failure) => {
            assert_eq!(failure.code, code, "{}", failure.message);
            assert!(failure.message.contains(name), "{}", failure.message);
        }
    }
}

#[test]
fn captured_record_import_preserves_archive_identity_and_metadata_without_facts() {
    let environment = import(request(MANIFEST, lock(false)));
    assert_eq!(environment.packages.len(), 1);
    let record = &environment.packages[0];
    assert_eq!(record.name, "ca-certificates");
    assert_eq!(record.version, "2024.2.2");
    assert_eq!(record.build, "hbcca054_0");
    assert_eq!(record.url, CERT);
    assert_eq!(record.sha256, CERT_SHA);
    assert_eq!(record.size, Some(155432));
    assert_eq!(record.timestamp, Some(1706843687645));
    assert_eq!(
        record.md5.as_deref(),
        Some("2f4327a1cbe7f022401b236e915a5fef")
    );
    assert_eq!(record.license.as_deref(), Some("ISC"));
    assert_eq!(record.purls, Some(BTreeSet::new()));
    assert!(environment.virtual_packages.is_empty());
    assert!(environment.system_requirements.virtual_packages.is_empty());
}

#[test]
fn canonical_selection_honors_features_targets_no_default_and_constraints() {
    let manifest = "[workspace]\nname='selected'\nchannels=['conda-forge']\nplatforms=['linux-64','osx-arm64']\n[dependencies]\npython='>=3.12'\n[feature.certs.dependencies]\nca-certificates='>=2020'\n[feature.certs.target.linux-64.dependencies]\nca-certificates={version='>=2024.2,<2025', build='hbcca054_*'}\n[feature.certs.constraints]\nca-certificates='<2025'\n[environments]\ncerts={features=['certs'], no-default-feature=true}\n";
    let captured_lock = lock(false).replace("  default:", "  certs:");
    let mut selected = request(manifest, captured_lock.clone());
    selected.environment = "certs".into();
    assert_eq!(import(selected.clone()).packages[0].sha256, CERT_SHA);
    selected.manifest_bytes =
        manifest.replace("ca-certificates='<2025'", "ca-certificates='<2024'");
    refuses(selected, "unsatisfied_manifest", "ca-certificates");
    let mut with_default = request(
        &manifest.replace("no-default-feature=true", "no-default-feature=false"),
        captured_lock,
    );
    with_default.environment = "certs".into();
    refuses(with_default, "unsatisfied_manifest", "python");
}

#[test]
fn stale_manifest_platform_environment_and_channels_refuse() {
    refuses(
        request(&MANIFEST.replace(">=2024.2,<2025", ">=2025"), lock(false)),
        "unsatisfied_manifest",
        "ca-certificates",
    );
    refuses(
        request(&MANIFEST.replace("hbcca054_*", "other_*"), lock(false)),
        "unsatisfied_manifest",
        "ca-certificates",
    );
    let mut wrong_platform = request(MANIFEST, lock(false));
    wrong_platform.platform = "osx-arm64".into();
    refuses(wrong_platform, "unknown_platform", "osx-arm64");
    let mut wrong_env = request(MANIFEST, lock(false));
    wrong_env.environment = "missing".into();
    refuses(wrong_env, "unknown_environment", "missing");
    refuses(
        request(&MANIFEST.replace("conda-forge", "bioconda"), lock(false)),
        "unsatisfied_manifest",
        "channel",
    );
}

#[test]
fn extra_missing_transitive_and_constrained_records_refuse() {
    refuses(request(MANIFEST, lock(true)), "invalid_closure", "tzdata");
    let needs_tz = lock(false).replace(
        "  license: ISC",
        "  depends:\n  - tzdata >=2024a\n  license: ISC",
    );
    refuses(
        request(MANIFEST, needs_tz),
        "unsatisfied_manifest",
        "tzdata",
    );
    let constrained = lock(true).replace(
        "  license: ISC",
        "  depends:\n  - tzdata >=2024a\n  constrains:\n  - tzdata <2023\n  license: ISC",
    );
    refuses(
        request(MANIFEST, constrained),
        "unsatisfied_manifest",
        "tzdata",
    );
    let transitive = lock(true).replace(
        "  license: ISC",
        "  depends:\n  - tzdata >=2024a\n  license: ISC",
    );
    assert_eq!(
        import(request(MANIFEST, transitive))
            .packages
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["ca-certificates", "tzdata"]
    );
}

#[test]
fn sha256_is_mandatory_and_authenticated_inputs_are_refused() {
    refuses(
        request(
            MANIFEST,
            lock(false).replace(&format!("  sha256: {CERT_SHA}\n"), ""),
        ),
        "md5_only",
        "ca-certificates",
    );
    for secret_channel in [
        "https://user:password@conda.anaconda.org/conda-forge",
        "https://conda.anaconda.org/t/secret/conda-forge",
    ] {
        refuses(
            request(
                &MANIFEST.replace("conda-forge", secret_channel),
                lock(false),
            ),
            "credentials_rejected",
            "authenticated",
        );
    }
    refuses(
        request(
            MANIFEST,
            lock(false).replace(
                "https://conda.anaconda.org/conda-forge/",
                "https://user:password@conda.anaconda.org/conda-forge/",
            ),
        ),
        "credentials_rejected",
        "authenticated",
    );
}

#[test]
fn named_conda_source_records_are_refused_before_conversion() {
    // Upstream tests/data/satisfiability/source-dependency/pixi.lock.
    let source_lock = "version: 6\nenvironments:\n  default:\n    channels:\n    - url: https://prefix.dev/pixi-build-backends/\n    - url: https://prefix.dev/conda-forge/\n    packages:\n      linux-64:\n      - conda: child-package\npackages:\n- conda: child-package\n  name: child-package\n  version: 0.1.0\n  build: ''\n  subdir: noarch\n  noarch: false\n  variants:\n    target_platform: noarch\n";
    refuses(
        request(MANIFEST, source_lock.into()),
        "source_package",
        "child-package",
    );
}

// Unmodified upstream wheel-with-correct-tags fixture from the same pinned
// Pixi revision above: CPython 3.14, abi3 cryptography, cffi and pycparser.
const WHEEL_MANIFEST: &str = include_str!("fixtures/pixi.toml");
const WHEEL_LOCK: &str = include_str!("fixtures/pixi.lock");

#[test]
fn real_wheel_lock_preserves_complete_selected_metadata_and_archives() {
    let environment = import(request(WHEEL_MANIFEST, WHEEL_LOCK.into()));
    assert_eq!(
        environment
            .wheels
            .iter()
            .map(|wheel| wheel.name.as_str())
            .collect::<Vec<_>>(),
        ["cffi", "cryptography", "pycparser"]
    );
    let cryptography = &environment.wheels[1];
    assert_eq!(cryptography.version, "46.0.3");
    assert_eq!(
        cryptography.sha256,
        "01ca9ff2885f3acc98c29f1860552e37f6d7c7d013d7334ff2a9de43a449315d"
    );
    assert!(cryptography.filename.contains("cp311-abi3"));
    assert!(cryptography.url.ends_with(&cryptography.filename));
    assert!(cryptography.requires_python.is_some());
    assert!(
        cryptography
            .requires_dist
            .iter()
            .any(|requirement| requirement.starts_with("cffi"))
    );
    assert!(
        cryptography
            .requires_dist
            .iter()
            .any(|requirement| requirement.contains("extra =="))
    );
    assert!(
        environment
            .packages
            .iter()
            .any(|package| package.name == "python" && package.version == "3.14.2")
    );
}

#[test]
fn wheel_hash_source_and_editable_admission_names_the_record() {
    refuses(
        request(
            WHEEL_MANIFEST,
            WHEEL_LOCK.replace(
                "  sha256: 01ca9ff2885f3acc98c29f1860552e37f6d7c7d013d7334ff2a9de43a449315d\n",
                "",
            ),
        ),
        "missing_hash",
        "cryptography",
    );
    let old = "cryptography-46.0.3-cp311-abi3-manylinux2014_x86_64.manylinux_2_17_x86_64.whl";
    refuses(
        request(
            WHEEL_MANIFEST,
            WHEEL_LOCK.replace(old, "cryptography-46.0.3.tar.gz"),
        ),
        "source_package",
        "cryptography",
    );
    for editable in [false, true] {
        let source = format!(
            "version: 6\nenvironments:\n  default:\n    channels: []\n    packages:\n      linux-64:\n      - pypi: ./foo\npackages:\n- pypi: ./foo\n  name: foo\n  version: 0.1.0\n  editable: {editable}\n"
        );
        refuses(request(WHEEL_MANIFEST, source), "source_package", "foo");
    }
    refuses(
        request(
            &WHEEL_MANIFEST.replace(
                "cryptography = \">=46.0.3, <47\"",
                "cryptography = { path = './cryptography', editable = true }",
            ),
            WHEEL_LOCK.into(),
        ),
        "source_package",
        "cryptography",
    );
}

#[test]
fn incompatible_wheel_tags_and_python_requirements_refuse() {
    for (lock, name) in [
        (
            WHEEL_LOCK.replace("cffi-2.0.0-cp314-cp314", "cffi-2.0.0-cp313-cp313"),
            "cffi",
        ),
        (
            WHEEL_LOCK.replace(
                "manylinux2014_x86_64.manylinux_2_17_x86_64",
                "macosx_11_0_arm64",
            ),
            "cffi",
        ),
        (
            WHEEL_LOCK.replace("requires_python: '>=3.9'", "requires_python: '>=3.15'"),
            "cffi",
        ),
    ] {
        let failure = import_pixi(request(WHEEL_MANIFEST, lock)).unwrap_err();
        assert!(failure.message.contains(name), "{}", failure.message);
    }
}

#[test]
fn wheel_roots_transitive_versions_extras_and_indexes_must_satisfy_manifest() {
    refuses(
        request(
            &WHEEL_MANIFEST.replace(">=46.0.3, <47", ">=47"),
            WHEEL_LOCK.into(),
        ),
        "unsatisfied_manifest",
        "cryptography",
    );
    refuses(
        request(
            WHEEL_MANIFEST,
            WHEEL_LOCK.replace("cffi>=2.0.0", "cffi>=3.0.0"),
        ),
        "unsatisfied_manifest",
        "cffi",
    );
    refuses(
        request(
            &WHEEL_MANIFEST.replace(
                "cryptography = \">=46.0.3, <47\"",
                "cryptography = { version = '>=46.0.3, <47', extras = ['ssh'] }",
            ),
            WHEEL_LOCK.into(),
        ),
        "unsatisfied_manifest",
        "bcrypt",
    );
    refuses(
        request(
            &format!("{WHEEL_MANIFEST}\n[pypi-options]\nindex-url='https://example.org/simple'\n"),
            WHEEL_LOCK.into(),
        ),
        "unsatisfied_manifest",
        "indexes",
    );
}

#[test]
fn old_locks_preserve_explicit_unused_system_policy_without_inventing_facts() {
    let manifest = format!(
        "{MANIFEST}\n[system-requirements]\nlinux='5.10'\nlibc={{family='glibc', version='2.28'}}\narchspec='x86_64_v3'\n"
    );
    let environment = import(request(&manifest, lock(false)));
    assert!(environment.virtual_packages.is_empty());
    assert_eq!(
        environment.system_requirements.archspec.as_deref(),
        Some("x86_64_v3")
    );
    assert!(
        environment
            .system_requirements
            .virtual_packages
            .iter()
            .any(|requirement| requirement.name == "__linux"
                && requirement.minimum_version == "5.10")
    );
    assert!(
        environment
            .system_requirements
            .virtual_packages
            .iter()
            .any(|requirement| requirement.name == "__glibc"
                && requirement.minimum_version == "2.28")
    );
}

#[test]
fn pyproject_capture_uses_upstream_pixi_tables() {
    let manifest = "[project]\nname='captured'\nversion='1.0'\n[tool.pixi.workspace]\nchannels=['conda-forge']\nplatforms=['linux-64']\n[tool.pixi.feature.certs.dependencies]\nca-certificates='>=2024.2,<2025'\n[tool.pixi.environments]\ncerts={features=['certs'], no-default-feature=true}\n";
    let mut selected = request(manifest, lock(false).replace("  default:", "  certs:"));
    selected.environment = "certs".into();
    assert_eq!(import(selected).packages[0].sha256, CERT_SHA);
}

#[test]
fn legacy_v3_v4_v5_readers_keep_real_immutable_records() {
    let v3 = format!(
        "version: 3\nmetadata:\n  channels:\n  - url: https://conda.anaconda.org/conda-forge/\n  platforms: [linux-64]\npackage:\n- manager: conda\n  platform: linux-64\n  name: ca-certificates\n  version: 2024.2.2\n  build: hbcca054_0\n  url: {CERT}\n  hash:\n    sha256: {CERT_SHA}\n  size: 155432\n"
    );
    assert_eq!(import(request(MANIFEST, v3)).packages[0].sha256, CERT_SHA);
    for version in [4, 5] {
        let legacy = lock(false).replace("version: 6", &format!("version: {version}"))
            .replace(&format!("packages:\n- conda: {CERT}"), &format!("packages:\n- kind: conda\n  name: ca-certificates\n  version: 2024.2.2\n  build: hbcca054_0\n  subdir: linux-64\n  noarch: false\n  url: {CERT}"));
        assert_eq!(
            import(request(MANIFEST, legacy)).packages[0].sha256,
            CERT_SHA
        );
    }
}

#[test]
fn contradictory_archive_identity_and_platform_are_refused() {
    let record = lock(false).replace("  license: ISC", "  version: '2024.3'\n  license: ISC");
    refuses(
        request(MANIFEST, record),
        "invalid_closure",
        "immutable archive filename",
    );
    let record = lock(false).replace("  license: ISC", "  subdir: osx-arm64\n  license: ISC");
    refuses(request(MANIFEST, record), "invalid_closure", "osx-arm64");
}

#[test]
fn modern_partial_virtual_declarations_do_not_manufacture_default_facts() {
    let manifest = format!("{MANIFEST}\n[system-requirements]\ncuda='12'\n");
    let modern = lock(false)
        .replace(
            "version: 6",
            "version: 7\nplatforms:\n- name: linux-64\n  virtual-packages:\n  - __cuda=12",
        )
        .replace(
            "  license: ISC",
            "  depends:\n  - __glibc >=2.17\n  - __cuda >=12\n  license: ISC",
        );
    let environment = import(request(&manifest, modern.clone()));
    assert_eq!(
        environment.virtual_packages,
        [LockedVirtualPackage {
            name: "__cuda".into(),
            version: "12".into(),
            build: String::new()
        }]
    );
    assert_eq!(
        environment.packages[0].depends,
        ["__glibc >=2.17", "__cuda >=12"]
    );
    refuses(
        request(&manifest, modern.replace("__cuda >=12", "__cuda >=13")),
        "unsatisfied_manifest",
        "__cuda",
    );
    refuses(
        request(&manifest.replace("cuda='12'", "cuda='13'"), modern),
        "unsatisfied_manifest",
        "virtual-package",
    );
}
