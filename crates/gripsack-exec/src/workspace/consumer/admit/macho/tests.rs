use super::*;
use gripsack_policy::target::TargetRequirement;

fn host() -> HostTarget {
    HostTarget {
        requirement: TargetRequirement {
            os: TargetOs::Macos,
            arch: TargetArch::Aarch64,
            abi: Some(BinaryAbi::Darwin),
            minimum_os: None,
        },
        os: TargetOs::Macos,
        arch: TargetArch::Aarch64,
        abi: Some(BinaryAbi::Darwin),
    }
}
fn span() -> Span {
    Span {
        file: "workspace.ts".into(),
        line: 1,
        col: None,
    }
}
fn object(path: &Path, executable: bool, needed: &[&str], rpaths: &[&str]) {
    let mut commands = Vec::new();
    let mut count = 0u32;
    let mut command = |kind: u32, name: &str, offset: usize| {
        let size = (offset + name.len() + 1).next_multiple_of(8);
        let mut bytes = vec![0; size];
        bytes[..4].copy_from_slice(&kind.to_le_bytes());
        bytes[4..8].copy_from_slice(&(size as u32).to_le_bytes());
        bytes[8..12].copy_from_slice(&(offset as u32).to_le_bytes());
        bytes[offset..offset + name.len()].copy_from_slice(name.as_bytes());
        commands.extend(bytes);
        count += 1;
    };
    if executable {
        command(0xe, "/usr/lib/dyld", 12);
    }
    for name in needed {
        command(0xc, name, 24);
    }
    for path in rpaths {
        command(0x8000_001c, path, 12);
    }
    let mut bytes = vec![0; 32];
    bytes[..4].copy_from_slice(&[0xcf, 0xfa, 0xed, 0xfe]);
    bytes[4..8].copy_from_slice(&0x0100_000cu32.to_le_bytes());
    bytes[12..16].copy_from_slice(&(if executable { 2u32 } else { 6 }).to_le_bytes());
    bytes[16..20].copy_from_slice(&count.to_le_bytes());
    bytes[20..24].copy_from_slice(&(commands.len() as u32).to_le_bytes());
    bytes.extend(commands);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}
fn closure<'a>(root: &Path, host: &'a HostTarget, span: &'a Span) -> Closure<'a> {
    Closure {
        roots: BTreeMap::from([(root.to_owned(), false)]),
        objects: BTreeMap::new(),
        host,
        span,
    }
}
fn plan(root: &Path) -> Result<Vec<PathBuf>, ExecError> {
    let host = host();
    let span = span();
    let mut closure = closure(root, &host, &span);
    let main = root.join("bin/main");
    closure.load(&main, true)?;
    closure.inventory()?;
    closure.plan(&main)
}

#[test]
fn nested_loader_origin_and_transitive_dependencies_are_checked() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    object(
        &root.join("bin/main"),
        true,
        &["@executable_path/../lib/middle.dylib"],
        &[],
    );
    object(
        &root.join("lib/middle.dylib"),
        false,
        &["@loader_path/deep/inner.dylib"],
        &[],
    );
    assert!(plan(&root).is_err(), "missing transitive object");
    object(
        &root.join("lib/deep/inner.dylib"),
        false,
        &["/usr/lib/libSystem.B.dylib"],
        &[],
    );
    assert_eq!(
        plan(&root).unwrap().into_iter().collect::<BTreeSet<_>>(),
        BTreeSet::from([root.join("lib"), root.join("lib/deep")])
    );
    let mut bytes = std::fs::read(root.join("lib/deep/inner.dylib")).unwrap();
    bytes[4..8].copy_from_slice(&0x0100_0007u32.to_le_bytes());
    std::fs::write(root.join("lib/deep/inner.dylib"), bytes).unwrap();
    assert!(plan(&root).is_err(), "wrong machine in transitive object");
}

#[test]
fn escaped_symlink_and_system_prefix_impostor_are_refused() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let outside = tempfile::tempdir().unwrap();
    object(
        &root.join("bin/main"),
        true,
        &["@loader_path/../lib/inner.dylib"],
        &[],
    );
    object(&outside.path().join("inner.dylib"), false, &[], &[]);
    std::fs::create_dir(root.join("lib")).unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("inner.dylib"),
        root.join("lib/inner.dylib"),
    )
    .unwrap();
    assert!(plan(&root).is_err());
    object(
        &root.join("bin/main"),
        true,
        &["/usr/lib/not-an-apple-runtime.dylib"],
        &[],
    );
    assert!(plan(&root).is_err());
    object(
        &root.join("bin/main"),
        true,
        &["/usr/lib/../local/lib/libSystem.B.dylib"],
        &[],
    );
    assert!(plan(&root).is_err());
}

#[test]
fn rpath_ancestry_and_sealed_basename_collisions_are_not_ambient_fallback() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    object(
        &root.join("bin/main"),
        true,
        &["@rpath/middle.dylib"],
        &["@executable_path/../lib"],
    );
    object(
        &root.join("lib/middle.dylib"),
        false,
        &["@rpath/inner.dylib"],
        &[],
    );
    object(&root.join("lib/inner.dylib"), false, &[], &[]);
    assert_eq!(plan(&root).unwrap(), vec![root.join("lib")]);
    object(
        &root.join("plugins/extension.so"),
        false,
        &["@loader_path/inner.dylib"],
        &[],
    );
    object(&root.join("plugins/inner.dylib"), false, &[], &[]);
    assert!(
        plan(&root).is_err(),
        "flattening two different inner.dylib selections changes lookup"
    );
}

#[test]
fn ambiguous_rpaths_and_escaping_search_directories_are_refused() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    object(
        &root.join("bin/main"),
        true,
        &["@rpath/inner.dylib"],
        &["@loader_path/../lib", "@loader_path/../other"],
    );
    object(&root.join("lib/inner.dylib"), false, &[], &[]);
    object(&root.join("other/inner.dylib"), false, &[], &[]);
    assert!(plan(&root).is_err());
    object(
        &root.join("bin/main"),
        true,
        &[],
        &["@loader_path/../../outside"],
    );
    assert!(plan(&root).is_err());
}

#[test]
fn fixed_prefix_absolute_paths_require_materialized_authority() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let library = root.join("lib/inner.dylib");
    object(
        &root.join("bin/main"),
        true,
        &[library.to_str().unwrap()],
        &[],
    );
    object(&library, false, &[], &[]);
    assert!(plan(&root).is_err());
    let host = host();
    let span = span();
    let mut closure = closure(&root, &host, &span);
    closure.roots.insert(root.clone(), true);
    closure.load(&root.join("bin/main"), true).unwrap();
    assert_eq!(
        closure.plan(&root.join("bin/main")).unwrap(),
        vec![root.join("lib")]
    );
    // macOS commonly spells /private/var through the /var directory alias.
    let aliases = tempfile::tempdir().unwrap();
    let alias = aliases.path().join("final prefix");
    std::os::unix::fs::symlink(&root, &alias).unwrap();
    object(
        &root.join("bin/main"),
        true,
        &[alias.join("lib/inner.dylib").to_str().unwrap()],
        &[],
    );
    closure.objects.clear();
    closure.load(&root.join("bin/main"), true).unwrap();
    assert_eq!(
        closure.plan(&root.join("bin/main")).unwrap(),
        vec![root.join("lib")]
    );
}

#[test]
fn header_and_required_commands_fail_closed() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let main = root.join("bin/main");
    object(&main, true, &[], &[]);
    let original = std::fs::read(&main).unwrap();
    for (offset, word) in [(4, 0x0100_0007u32), (8, 2), (24, 0x100), (32, 0x8000_00ff)] {
        let mut bytes = original.clone();
        bytes[offset..offset + 4].copy_from_slice(&word.to_le_bytes());
        std::fs::write(&main, bytes).unwrap();
        assert!(
            plan(&root).is_err(),
            "header/load-command mutation at {offset}"
        );
    }
}

#[test]
fn retained_runtime_roots_are_required_for_cross_package_origins() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let app = root.join("app");
    let runtime = root.join("runtime");
    let main = app.join("bin/main");
    object(
        &main,
        true,
        &["@loader_path/../../runtime/lib/inner.dylib"],
        &[],
    );
    object(&runtime.join("lib/inner.dylib"), false, &[], &[]);
    let host = host();
    let span = span();
    let mut selected = closure(&app, &host, &span);
    selected.load(&main, true).unwrap();
    assert!(
        selected.plan(&main).is_err(),
        "an undeclared sibling is not retained authority"
    );
    selected.roots.insert(runtime.clone(), false);
    assert_eq!(selected.plan(&main).unwrap(), vec![runtime.join("lib")]);
}

#[test]
fn required_dylib_commands_and_command_extent_cannot_hide_dependencies() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let library = root.join("library.dylib");
    object(&library, false, &["@loader_path/missing.dylib"], &[]);
    let original = std::fs::read(&library).unwrap();
    for kind in [0x8000_0018u32, 0x8000_001f, 0x8000_0023] {
        let mut bytes = original.clone();
        bytes[32..36].copy_from_slice(&kind.to_le_bytes());
        let metadata = classify(&mut std::io::Cursor::new(&bytes)).unwrap();
        assert_eq!(
            metadata.needed_libraries,
            vec![std::ffi::OsString::from("@loader_path/missing.dylib")]
        );
        // Truncating the declared command region must fail even when the
        // underlying file still contains all of the command's bytes.
        bytes[20..24].copy_from_slice(&8u32.to_le_bytes());
        assert!(classify(&mut std::io::Cursor::new(bytes)).is_err());
    }
}

#[test]
fn minimum_macos_floor_requires_measured_compatible_host() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let main = root.join("bin/main");
    object(&main, true, &[], &[]);
    let mut bytes = std::fs::read(&main).unwrap();
    let length = u32::from_le_bytes(bytes[20..24].try_into().unwrap());
    bytes[16..20].copy_from_slice(&2u32.to_le_bytes());
    bytes[20..24].copy_from_slice(&(length + 16).to_le_bytes());
    for word in [0x24u32, 16, 0x000e_0100, 0x000e_0100] {
        bytes.extend(word.to_le_bytes());
    }
    std::fs::write(&main, bytes).unwrap();
    assert!(
        plan(&root).is_err(),
        "unknown actual host cannot satisfy an OS floor"
    );
    let mut host = host();
    let span = span();
    host.requirement.minimum_os = Some(OsRelease {
        major: 14,
        minor: 0,
        patch: 9,
    });
    assert!(closure(&root, &host, &span).load(&main, true).is_err());
    host.requirement.minimum_os = Some(OsRelease {
        major: 14,
        minor: 1,
        patch: 0,
    });
    let mut selected = closure(&root, &host, &span);
    selected.load(&main, true).unwrap();
    assert_eq!(selected.plan(&main).unwrap(), Vec::<PathBuf>::new());
}

#[test]
fn hardened_runtime_signatures_refuse_dyld_translation() {
    let mut bytes = Vec::new();
    for word in [
        0xfade0cc0u32,
        36,
        1,
        0,
        20,
        0xfade0c02,
        16,
        0x20000,
        0x10000,
    ] {
        bytes.extend(word.to_be_bytes());
    }
    assert!(signature_controls(&bytes, true, &span()).is_err());
    bytes[32..36].copy_from_slice(&2u32.to_be_bytes()); // ad-hoc, not hardened
    signature_controls(&bytes, true, &span()).unwrap();
    bytes[16..20].copy_from_slice(&0xfffffff0u32.to_be_bytes());
    assert!(signature_controls(&bytes, true, &span()).is_err());
}

#[cfg(target_os = "macos")]
fn launch(path: &Path, libraries: &[PathBuf], args: &[&std::ffi::OsStr], cwd: &Path) {
    use gripsack_process::{
        Control, EnvironmentOverlay, Invocation, Limits, NativeInput, OperatorEnvironment,
        ProcessRole, SelectedProgram,
    };
    let environment =
        OperatorEnvironment::admit([("PATH".into(), "/usr/bin:/bin".into())]).unwrap();
    let limits = Limits {
        timeout: std::time::Duration::from_secs(60),
        ..Limits::default()
    };
    let deadline = std::time::Instant::now() + limits.timeout;
    let selected = SelectedProgram::select(&environment, path, None, deadline)
        .unwrap()
        .with_macho_libraries(libraries)
        .unwrap();
    let overlay =
        EnvironmentOverlay::admit([("PYTHONDONTWRITEBYTECODE".into(), "1".into())], [], [])
            .unwrap();
    let outcome = Invocation::admit(&environment, ProcessRole::Task, &selected, cwd, limits)
        .unwrap()
        .with_overlay(overlay)
        .run(args, NativeInput::Bytes(b""), None, |_| Control::Continue)
        .unwrap();
    assert!(
        outcome.success,
        "{:?}: {}",
        outcome.receipt,
        String::from_utf8_lossy(&outcome.stderr)
    );
}

#[cfg(target_os = "macos")]
fn native_plan(root: &Path, main: &Path) -> Vec<PathBuf> {
    let mut host = host();
    if cfg!(target_arch = "x86_64") {
        host.arch = TargetArch::X86_64;
        host.requirement.arch = host.arch;
    }
    host.requirement.minimum_os = Some(crate::facts::platform_release().unwrap().floor);
    let span = span();
    let mut closure = closure(root, &host, &span);
    closure.roots.insert(root.to_owned(), true);
    closure.load(main, true).unwrap();
    closure.inventory().unwrap();
    closure.plan(main).unwrap()
}

/// Requires Xcode CLI tools, not Virtualization.framework or a Linux VM.
#[cfg(target_os = "macos")]
#[test]
fn native_sealed_macho_launch_preserves_origin_and_extension_closure() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let staging = root.join("staging");
    for path in [
        &staging,
        &root.join("bin"),
        &root.join("lib/deep"),
        &root.join("plugins"),
    ] {
        std::fs::create_dir_all(path).unwrap();
    }
    let compile = |source: &str, name: &str, arguments: &[&str], output: &Path| {
        let source_path = staging.join(name);
        std::fs::write(&source_path, source).unwrap();
        let result = std::process::Command::new("/usr/bin/cc")
            .args(arguments)
            .arg(&source_path)
            .arg("-o")
            .arg(output)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    };
    let inner = root.join("lib/deep/inner.dylib");
    compile(
        "int inner(void) { return 42; }",
        "inner.c",
        &[
            "-dynamiclib",
            "-Wl,-install_name,@loader_path/deep/inner.dylib",
        ],
        &inner,
    );
    let middle = root.join("lib/middle.dylib");
    compile(
        "extern int inner(void); int middle(void) { return inner(); }",
        "middle.c",
        &[
            "-dynamiclib",
            "-Wl,-install_name,@rpath/middle.dylib",
            inner.to_str().unwrap(),
        ],
        &middle,
    );
    let extension = root.join("plugins/extension.so");
    compile(
        "extern int middle(void); int extension(void) { return middle(); }",
        "extension.c",
        &["-bundle", middle.to_str().unwrap()],
        &extension,
    );
    let main = root.join("bin/main");
    compile(
        "#include <dlfcn.h>\n#include <stdio.h>\nextern int middle(void); int main(int argc, char** argv) { void *h = dlopen(argv[1], RTLD_NOW); if (!h) { fputs(dlerror(), stderr); return 2; } int (*f)(void) = dlsym(h, \"extension\"); if (!f || f()!=42 || middle()!=42) return 3; FILE *p=fopen(\"proof\",\"w\"); fputs(\"42\",p); return fclose(p); }",
        "main.c",
        &[
            "-Wl,-rpath,@executable_path/../lib",
            middle.to_str().unwrap(),
        ],
        &main,
    );
    let libraries = native_plan(&root, &main);
    std::fs::remove_dir_all(staging).unwrap();
    launch(&main, &libraries, &[extension.as_os_str()], &root);
    assert_eq!(std::fs::read(root.join("proof")).unwrap(), b"42");
}

/// The caller supplies a genuinely materialized frozen Python+pip prefix and
/// deletes its staging/package caches first. This is a native loader smoke,
/// not a replacement for the end-to-end Conda publication/receipt harness.
#[cfg(target_os = "macos")]
#[test]
#[ignore = "requires GRIPSACK_TEST_CONDA_PREFIX after frozen materialization and staging deletion"]
fn native_conda_python_and_noarch_entrypoint() {
    let root = PathBuf::from(std::env::var_os("GRIPSACK_TEST_CONDA_PREFIX").expect("final prefix"))
        .canonicalize()
        .unwrap();
    let before = gripsack_store::canonical_tree_hash(&root).unwrap();
    let python = root.join("bin/python").canonicalize().unwrap();
    let libraries = native_plan(&root, &python);
    let cwd = tempfile::tempdir().unwrap();
    launch(
        &python,
        &libraries,
        &[
            std::ffi::OsStr::new("-c"),
            std::ffi::OsStr::new(
                "import _ssl, _sqlite3, zlib, sys, pathlib; assert sys.dont_write_bytecode; assert pathlib.Path(sys.prefix).resolve() == pathlib.Path(sys.argv[1]).resolve(); pathlib.Path('python-proof').write_text(_ssl.OPENSSL_VERSION)",
            ),
            root.as_os_str(),
        ],
        cwd.path(),
    );
    assert!(
        std::fs::read_to_string(cwd.path().join("python-proof"))
            .unwrap()
            .starts_with("OpenSSL")
    );
    launch(
        &root.join("bin/pip"),
        &libraries,
        &[std::ffi::OsStr::new("--version")],
        cwd.path(),
    );
    assert_eq!(
        gripsack_store::canonical_tree_hash(&root).unwrap(),
        before,
        "native Python/entrypoint execution must not mutate the published prefix"
    );
}

#[test]
fn loader_origin_resolves_symlinks_before_parent_components() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    object(
        &root.join("bin/main"),
        true,
        &["@loader_path/link/../inner.dylib"],
        &[],
    );
    object(&root.join("bin/inner.dylib"), false, &[], &[]);
    object(&root.join("other/inner.dylib"), false, &[], &[]);
    std::fs::create_dir(root.join("other/deep")).unwrap();
    std::os::unix::fs::symlink(root.join("other/deep"), root.join("bin/link")).unwrap();
    assert_eq!(plan(&root).unwrap(), vec![root.join("other")]);
}
