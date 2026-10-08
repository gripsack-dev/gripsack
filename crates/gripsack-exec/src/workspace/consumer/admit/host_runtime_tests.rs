//! Controlled real ELF regression: no network, private binary or solver fixtures.
use super::*;
use crate::workspace::artifact::{Artifact, ProvidedCommand};
use gripsack_ir::workspace::{PlatformAbi, PlatformArch, PlatformOs};
use gripsack_process::{Control, Invocation, Limits, NativeInput, ProcessRole, Sha256Digest};
use std::{ffi::OsStr, sync::Arc, time::Instant};

fn compile(root: &Path, name: &str, source: &str, arguments: &[&str], output: &Path) {
    let path = root.join(name);
    std::fs::write(&path, source).unwrap();
    let result = std::process::Command::new("/usr/bin/cc")
        .arg(&path)
        .args(arguments)
        .arg("-o")
        .arg(output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn compiled_host_runpath_requires_scoped_policy_and_readmits_each_launch() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    std::fs::create_dir(root.join("store")).unwrap();
    let payload = gripsack_store::content_path(&root, "workspace-recipe", &"1".repeat(64));
    let libraries = root.join("host-libraries");
    let wrong = root.join("wrong");
    for path in [&payload, &libraries, &wrong] {
        std::fs::create_dir(path).unwrap();
    }
    let library = libraries.join("libreview.so");
    compile(
        &root,
        "library.c",
        "int review(void) { return 42; }",
        &["-shared", "-fPIC", "-Wl,-soname,libreview.so"],
        &library,
    );
    let program = payload.join("review");
    compile(
        &root,
        "main.c",
        "#include <stdio.h>\nextern int review(void); int main(int argc, char **argv) { printf(\"review=%d\\n\", review()); return argc > 1 ? 7 : review() != 42; }",
        &[
            library.to_str().unwrap(),
            &format!("-Wl,-rpath,{}", libraries.display()),
        ],
        &program,
    );
    let executable =
        ExecutableDigest::parse(&Sha256Digest::of(&std::fs::read(&program).unwrap()).to_string())
            .unwrap();
    let mut package = Package {
        identity: serde_json::from_str(&format!("\"{}\"", "0".repeat(64))).unwrap(),
        root: gripsack_store::content_path(&root, "workspace-package", &"0".repeat(64)),
        producer: Arc::new(Artifact {
            root: payload.clone(),
            payload: payload.clone(),
            tree: gripsack_store::canonical_tree_hash(&payload).unwrap(),
            retention: BTreeSet::new(),
        }),
        commands: BTreeMap::from([(
            "review".into(),
            ProvidedCommand {
                selector: "review".into(),
                executable,
            },
        )]),
        runtime: Vec::new(),
        target: WorkspacePlatform {
            os: PlatformOs::Linux,
            arch: if cfg!(target_arch = "x86_64") {
                PlatformArch::X86_64
            } else {
                PlatformArch::Aarch64
            },
            abi: Some(PlatformAbi::Gnu),
            minimum_os: None,
        },
        layout: CatalogPackageLayout::Relocatable,
        conda: None,
        host_runtime: None,
    };
    let facts = crate::facts::detect();
    let facts = gripsack_ir::HostFacts {
        os: facts.os.into(),
        arch: facts.arch.into(),
        libc: facts.libc.clone(),
        tags: vec![],
    };
    let limits = Limits::default();
    let deadline = Instant::now() + limits.timeout;
    let context = NativeContext::new(&facts, &root, deadline).unwrap();
    let span = Span {
        file: "host-runtime.ts".into(),
        line: 1,
        col: None,
    };
    let admit = |package: &Package| admit_command(package, "review", &executable, &context, &span);
    assert!(
        admit(&package).is_err(),
        "undeclared absolute RUNPATH must fail closed"
    );
    let policy = |path: &Path| {
        serde_json::from_value(serde_json::json!({"library_directories": [path]})).unwrap()
    };
    package.host_runtime = Some(policy(&wrong));
    assert!(
        admit(&package).is_err(),
        "an unrelated reviewed directory is not authority"
    );
    package.host_runtime = Some(policy(&libraries));
    // Persist the real package's v2 receipt, then replay it independently of
    // source declarations, exactly as a retained hook/managed launcher does.
    std::fs::create_dir(&package.root).unwrap();
    let receipt_path = package.root.join("package.json");
    let receipt = serde_json::json!({
        "version": 2, "package": package.identity,
        "producer_root": payload, "producer_payload": payload,
        "tree": package.producer.tree, "commands": package.commands,
        "runtime": {}, "target": package.target, "layout": package.layout,
        "host_runtime": package.host_runtime
    });
    std::fs::write(&receipt_path, serde_json::to_vec(&receipt).unwrap()).unwrap();
    let mut receipts = BTreeMap::new();
    crate::workspace::artifact::hook::capture(&root, &package, &mut receipts).unwrap();
    let digest = receipts[&package.identity];
    let restored =
        crate::workspace::artifact::hook::restore_launcher(&root, package.identity, digest)
            .unwrap();
    assert!(admit(&restored).is_ok());
    assert!(
        crate::workspace::artifact::hook::restore_launcher(
            &root,
            package.identity,
            Sha256Digest::of(b"wrong")
        )
        .is_err()
    );
    let mut changed = receipt.clone();
    changed["host_runtime"]["library_directories"] = serde_json::json!([wrong]);
    std::fs::write(&receipt_path, serde_json::to_vec(&changed).unwrap()).unwrap();
    assert!(
        crate::workspace::artifact::hook::restore_launcher(&root, package.identity, digest)
            .is_err()
    );
    std::fs::write(&receipt_path, serde_json::to_vec(&receipt).unwrap()).unwrap();
    let admitted = admit(&package).unwrap();
    let environment =
        gripsack_process::OperatorEnvironment::admit([("PATH".into(), "/usr/bin:/bin".into())])
            .unwrap();
    let options = crate::workspace::BuildOptions {
        environment: &environment,
        bridge: None,
        worker: Default::default(),
        deadline,
    };
    let selected = crate::workspace::consumer::bind_admitted_program(&admitted, &options).unwrap();
    for (arguments, success) in [(vec![], true), (vec![OsStr::new("fail")], false)] {
        let mut stdout = Vec::new();
        let outcome = Invocation::admit(&environment, ProcessRole::Task, &selected, &root, limits)
            .unwrap()
            .run(&arguments, NativeInput::Bytes(b""), None, |bytes| {
                stdout.extend_from_slice(bytes);
                Control::Continue
            })
            .unwrap();
        assert_eq!(
            outcome.success,
            success,
            "{}",
            String::from_utf8_lossy(&outcome.stderr)
        );
        assert_eq!(outcome.receipt.exit_code, Some(if success { 0 } else { 7 }));
        assert_eq!(stdout, b"review=42\n");
    }
    // Reusing NativeContext must not let an earlier compatibility cache authorize
    // a newly introduced alias or capability shadow in the mutable host root.
    let outside = wrong.join("libreview.so");
    std::fs::rename(&library, &outside).unwrap();
    std::os::unix::fs::symlink(&outside, &library).unwrap();
    assert!(
        admit(&package).is_err(),
        "host dependency alias escapes reviewed root"
    );
    let replay =
        crate::workspace::artifact::hook::restore_launcher(&root, package.identity, digest)
            .unwrap();
    assert!(
        admit(&replay).is_err(),
        "receipt replay must not cache mutable host admission"
    );
    std::fs::remove_file(&library).unwrap();
    std::fs::rename(&outside, &library).unwrap();
    let shadow = libraries.join("tls");
    std::fs::create_dir(&shadow).unwrap();
    std::fs::copy(&library, shadow.join("libreview.so")).unwrap();
    assert!(
        admit(&package).is_err(),
        "legacy hwcap shadow must be re-admitted"
    );
    std::fs::remove_dir_all(shadow).unwrap();
    let alias = root.join("alias");
    std::os::unix::fs::symlink(&libraries, &alias).unwrap();
    package.host_runtime = Some(
        serde_json::from_value(serde_json::json!({"library_directories": [&libraries, &alias]}))
            .unwrap(),
    );
    assert!(
        admit(&package).is_err(),
        "canonical duplicate roots must not hide ordering"
    );
    let root_alias = root.join("root-alias");
    std::os::unix::fs::symlink("/", &root_alias).unwrap();
    package.host_runtime = Some(policy(&root_alias));
    assert!(
        admit(&package).is_err(),
        "canonical root alias must not grant the whole host"
    );
    package.host_runtime = Some(policy(&libraries));
    // A transitive object's own absolute path cannot enlarge the host policy.
    compile(
        &root,
        "library.c",
        "int review(void) { return 42; }",
        &[
            "-shared",
            "-fPIC",
            "-Wl,-soname,libreview.so",
            &format!("-Wl,-rpath,{}", wrong.display()),
        ],
        &library,
    );
    assert!(
        admit(&package).is_err(),
        "transitive absolute path escapes the reviewed roots"
    );
    for path in [
        format!("{}/$LIB", libraries.display()),
        format!("{};{}", libraries.display(), wrong.display()),
    ] {
        compile(
            &root,
            "library.c",
            "int review(void) { return 42; }",
            &[
                "-shared",
                "-fPIC",
                "-Wl,-soname,libreview.so",
                &format!("-Wl,-rpath,{path}"),
            ],
            &library,
        );
        assert!(
            admit(&package).is_err(),
            "transitive loader token/delimiter must be rejected before missing-path filtering"
        );
    }
    // GNU uses the loading SONAME alias's parent for a dependency's $ORIGIN.
    // Canonical byte containment alone would admit the outside libB below.
    let inner = libraries.join("libB.so");
    compile(
        &root,
        "inner.c",
        "int inner(void) { return 42; }",
        &["-shared", "-fPIC", "-Wl,-soname,libB.so"],
        &inner,
    );
    compile(
        &root,
        "outside.c",
        "int inner(void) { return 99; }",
        &["-shared", "-fPIC", "-Wl,-soname,libB.so"],
        &root.join("libB.so"),
    );
    let nested = libraries.join("sub");
    std::fs::create_dir(&nested).unwrap();
    let real_library = nested.join("libreview-real.so");
    compile(
        &root,
        "alias.c",
        "extern int inner(void); int review(void) { return inner(); }",
        &[
            "-shared",
            "-fPIC",
            "-Wl,-soname,libreview.so",
            "-Wl,--disable-new-dtags,-rpath,$ORIGIN/..",
            inner.to_str().unwrap(),
        ],
        &real_library,
    );
    std::fs::remove_file(&library).unwrap();
    std::os::unix::fs::symlink(&real_library, &library).unwrap();
    let uncontrolled = std::process::Command::new(&program)
        .env_clear()
        .output()
        .unwrap();
    assert_eq!(
        uncontrolled.stdout, b"review=99\n",
        "controlled fixture must expose the real loader's alias origin"
    );
    assert!(
        admit(&package).is_err(),
        "an in-root library alias must not authorize its outside $ORIGIN/.."
    );
    compile(
        &root,
        "alias.c",
        "extern int inner(void); int review(void) { return inner(); }",
        &[
            "-shared",
            "-fPIC",
            "-Wl,-soname,libreview.so",
            "-Wl,--disable-new-dtags,-rpath,$ORIGIN",
            inner.to_str().unwrap(),
        ],
        &real_library,
    );
    let admitted_alias = admit(&package).unwrap();
    let selected_alias =
        crate::workspace::consumer::bind_admitted_program(&admitted_alias, &options).unwrap();
    let mut stdout = Vec::new();
    let outcome = Invocation::admit(
        &environment,
        ProcessRole::Task,
        &selected_alias,
        &root,
        limits,
    )
    .unwrap()
    .run(&[], NativeInput::Bytes(b""), None, |bytes| {
        stdout.extend_from_slice(bytes);
        Control::Continue
    })
    .unwrap();
    assert!(
        outcome.success,
        "{}",
        String::from_utf8_lossy(&outcome.stderr)
    );
    assert_eq!(
        stdout, b"review=42\n",
        "safe aliases retain their loading-origin semantics"
    );

    // A policy-free executable may explicitly retain a runtime package whose
    // real ELF library needs that package's reviewed host runtime directories.
    let runtime_payload = gripsack_store::content_path(&root, "workspace-recipe", &"3".repeat(64));
    std::fs::create_dir_all(runtime_payload.join("lib")).unwrap();
    let bridge = runtime_payload.join("lib/libbridge.so");
    compile(
        &root,
        "bridge.c",
        "extern int review(void); int bridge(void) { return review(); }",
        &[
            "-shared",
            "-fPIC",
            "-Wl,-soname,libbridge.so",
            &format!("-Wl,-rpath,{}", libraries.display()),
            library.to_str().unwrap(),
        ],
        &bridge,
    );
    let through_runtime = payload.join("through-runtime");
    compile(
        &root,
        "through.c",
        "#include <stdio.h>\nextern int bridge(void); int main(void) { printf(\"%d\\n\", bridge()); return bridge()!=42; }",
        &[bridge.to_str().unwrap(), "-Wl,--allow-shlib-undefined"],
        &through_runtime,
    );
    package.host_runtime = None;
    package.runtime.push(Arc::new(Package {
        identity: serde_json::from_str(&format!("\"{}\"", "2".repeat(64))).unwrap(),
        root: gripsack_store::content_path(&root, "workspace-package", &"2".repeat(64)),
        producer: Arc::new(Artifact {
            root: runtime_payload.clone(),
            payload: runtime_payload.clone(),
            tree: gripsack_store::canonical_tree_hash(&runtime_payload).unwrap(),
            retention: BTreeSet::new(),
        }),
        commands: BTreeMap::new(),
        runtime: Vec::new(),
        target: package.target.clone(),
        layout: CatalogPackageLayout::Relocatable,
        conda: None,
        host_runtime: Some(policy(&libraries)),
    }));
    let shadow_payload = gripsack_store::content_path(&root, "workspace-recipe", &"5".repeat(64));
    std::fs::create_dir_all(shadow_payload.join("lib")).unwrap();
    compile(
        &root,
        "shadow-bridge.c",
        "int bridge(void) { return 99; }",
        &["-shared", "-fPIC", "-Wl,-soname,libbridge.so"],
        &shadow_payload.join("lib/libbridge.so"),
    );
    package.runtime.push(Arc::new(Package {
        identity: serde_json::from_str(&format!("\"{}\"", "4".repeat(64))).unwrap(),
        root: gripsack_store::content_path(&root, "workspace-package", &"4".repeat(64)),
        producer: Arc::new(Artifact {
            root: shadow_payload.clone(),
            payload: shadow_payload.clone(),
            tree: gripsack_store::canonical_tree_hash(&shadow_payload).unwrap(),
            retention: BTreeSet::new(),
        }),
        commands: BTreeMap::new(),
        runtime: Vec::new(),
        target: package.target.clone(),
        layout: CatalogPackageLayout::Relocatable,
        conda: None,
        host_runtime: None,
    }));
    let through_digest = ExecutableDigest::parse(
        &Sha256Digest::of(&std::fs::read(&through_runtime).unwrap()).to_string(),
    )
    .unwrap();
    let admitted_runtime = admit_command(
        &package,
        "through-runtime",
        &through_digest,
        &context,
        &span,
    )
    .unwrap();
    package.runtime.reverse();
    let reordered = admit_command(
        &package,
        "through-runtime",
        &through_digest,
        &context,
        &span,
    )
    .unwrap();
    assert_eq!(
        admitted_runtime.library_dirs, reordered.library_dirs,
        "live declaration order and digest-ordered receipt replay must choose identical retained libraries"
    );
    let selected_runtime =
        crate::workspace::consumer::bind_admitted_program(&reordered, &options).unwrap();
    let mut stdout = Vec::new();
    let outcome = Invocation::admit(
        &environment,
        ProcessRole::Task,
        &selected_runtime,
        &root,
        limits,
    )
    .unwrap()
    .run(&[], NativeInput::Bytes(b""), None, |bytes| {
        stdout.extend_from_slice(bytes);
        Control::Continue
    })
    .unwrap();
    assert!(
        outcome.success,
        "{}",
        String::from_utf8_lossy(&outcome.stderr)
    );
    assert_eq!(stdout, b"42\n");
    package
        .runtime
        .retain(|package| package.host_runtime.is_some());
    Arc::get_mut(&mut package.runtime[0]).unwrap().host_runtime = None;
    assert!(
        admit_command(
            &package,
            "through-runtime",
            &through_digest,
            &context,
            &span
        )
        .is_err(),
        "prior admission of a runtime child's policy must not become invocation-wide host authority"
    );
}
