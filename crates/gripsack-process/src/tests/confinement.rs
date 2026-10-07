//! Real invocation oracles: granted bytes remain accessible, actual canaries
//! are denied, and confinement preserves argv0/argument data. The unconfined
//! controls establish that a missing path cannot masquerade as kernel denial.
use crate::{
    Boundary, Control, Invocation, Limits, NativeInput, NativeOutcome, OperatorEnvironment,
    ProcessRole, Ruleset, SelectedProgram, RuntimeAccess,
};
use std::{
    ffi::OsStr,
    path::Path,
    time::{Duration, Instant},
};

fn invoke<const N: usize>(
    arguments: [&str; N],
    cwd: &Path,
    boundary: Option<Boundary>,
) -> (NativeOutcome, Vec<u8>) {
    let environment = OperatorEnvironment::capture().unwrap();
    let selected = SelectedProgram::select(
        &environment,
        Path::new("/bin/sh"),
        None,
        Instant::now() + Duration::from_secs(15),
    )
    .unwrap();
    let mut invocation = Invocation::admit(
        &environment,
        ProcessRole::Evaluator,
        &selected,
        cwd,
        Limits::default(),
    )
    .unwrap();
    if let Some(boundary) = boundary {
        invocation = invocation.confine(Ruleset::assemble(&boundary, false).unwrap());
    }
    let mut bytes = Vec::new();
    let outcome = invocation
        .run(
            &arguments.map(OsStr::new),
            NativeInput::Bytes(b""),
            None,
            |chunk| {
                bytes.extend_from_slice(chunk);
                Control::Continue
            },
        )
        .unwrap();
    (outcome, bytes)
}
fn sh_boundary(extra_read: &Path) -> Boundary {
    let environment = OperatorEnvironment::capture().unwrap();
    let mut boundary = Boundary::new();
    let access = RuntimeAccess::discover(&environment, Path::new("/bin/sh"), |_| Ok(())).unwrap();
    for file in access.files() {
        boundary = boundary.read_file(file).unwrap();
    }
    for root in access.directories() {
        boundary = boundary.read_beneath(root).unwrap();
    }
    boundary.read_beneath(extra_read).unwrap()
}

#[test]
fn confined_process_reads_granted_paths_but_not_outside_them() {
    let root = tempfile::tempdir().unwrap();
    let inside = root.path().join("inside");
    let outside = root.path().join("outside");
    std::fs::create_dir(&inside).unwrap();
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(inside.join("file"), b"granted\n").unwrap();
    std::fs::write(outside.join("secret"), b"credential-canary\n").unwrap();
    let (control, bytes) = invoke(["-c", "cat outside/secret"], root.path(), None);
    assert!(control.success, "{:?}", control.receipt);
    assert_eq!(bytes, b"credential-canary\n");
    #[cfg(target_os = "macos")]
    {
        let original = outside.join("secret").canonicalize().unwrap();
        let data_volume = Path::new("/System/Volumes/Data");
        let alias = if original.starts_with(data_volume) {
            original
        } else {
            data_volume.join(original.strip_prefix("/").unwrap())
        };
        let arguments = ["-c", "cat \"$1\"", "reader", alias.to_str().unwrap()];
        let (control, bytes) = invoke(arguments, root.path(), None);
        assert!(
            control.success,
            "the APFS data alias must reach the actual canary"
        );
        assert_eq!(bytes, b"credential-canary\n");
        let (denied, bytes) = invoke(arguments, root.path(), Some(sh_boundary(&inside)));
        assert!(
            !denied.success,
            "a broad /System grant exposed the data volume"
        );
        assert!(
            bytes.is_empty(),
            "canary escaped through its APFS data alias"
        );
    }
    let (allowed, bytes) = invoke(
        ["-c", "cat inside/file"],
        root.path(),
        Some(sh_boundary(&inside)),
    );
    assert!(
        allowed.success,
        "{:?} {}",
        allowed.receipt,
        String::from_utf8_lossy(&allowed.stderr)
    );
    assert_eq!(bytes, b"granted\n");
    let (denied, bytes) = invoke(
        ["-c", "cat outside/secret"],
        root.path(),
        Some(sh_boundary(&inside)),
    );
    assert!(!denied.success, "{:?}", denied.receipt);
    assert!(bytes.is_empty(), "denied content reached stdout");
}

#[test]
fn confined_process_writes_only_within_scratch_roots() {
    let root = tempfile::tempdir().unwrap();
    let scratch = root.path().join("scratch");
    std::fs::create_dir(&scratch).unwrap();
    let (control, _) = invoke(["-c", "printf control > outside"], root.path(), None);
    assert!(control.success);
    let boundary = sh_boundary(&scratch).read_write_beneath(&scratch).unwrap();
    let (denied, _) = invoke(
        [
            "-c",
            "printf written > scratch/file; printf changed > outside",
        ],
        root.path(),
        Some(boundary),
    );
    assert!(!denied.success, "{:?}", denied.receipt);
    assert_eq!(std::fs::read(scratch.join("file")).unwrap(), b"written");
    assert_eq!(
        std::fs::read(root.path().join("outside")).unwrap(),
        b"control"
    );
}

#[test]
fn confined_launch_preserves_argv_zero_and_argument_data() {
    let root = tempfile::tempdir().unwrap();
    let (outcome, bytes) = invoke(
        ["-c", "printf '%s' \"$0\""],
        root.path(),
        Some(sh_boundary(root.path())),
    );
    assert!(
        outcome.success,
        "{:?} {}",
        outcome.receipt,
        String::from_utf8_lossy(&outcome.stderr)
    );
    assert_eq!(bytes, b"/bin/sh");
    let (outcome, bytes) = invoke(
        [
            "-c",
            "printf '<%s>' \"$1\" \"$2\" \"$3\"",
            "body",
            "",
            "two words",
            "$(printf injected)",
        ],
        root.path(),
        Some(sh_boundary(root.path())),
    );
    assert!(
        outcome.success,
        "{:?} {}",
        outcome.receipt,
        String::from_utf8_lossy(&outcome.stderr)
    );
    assert_eq!(bytes, b"<><two words><$(printf injected)>");
}

#[test]
fn boundary_rejects_missing_or_non_directory_roots() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("missing");
    assert!(Boundary::new().read_beneath(&missing).is_err());
    assert!(!missing.exists());
    let file = root.path().join("file");
    std::fs::write(&file, b"data").unwrap();
    assert!(Boundary::new().read_beneath(&file).is_err());
}

#[test]
fn exact_file_grants_never_authorize_siblings() {
    let root = tempfile::tempdir().unwrap();
    let allowed = root.path().join("allowed");
    let denied = root.path().join("sibling");
    std::fs::write(&allowed, b"allowed\n").unwrap();
    std::fs::write(&denied, b"denied\n").unwrap();
    let environment = OperatorEnvironment::capture().unwrap();
    let access = RuntimeAccess::discover(&environment, Path::new("/bin/sh"), |_| Ok(())).unwrap();
    let boundary = || {
        let mut boundary = Boundary::new().read_file(&allowed).unwrap();
        for file in access.files() { boundary = boundary.read_file(file).unwrap(); }
        for directory in access.directories() { boundary = boundary.read_beneath(directory).unwrap(); }
        boundary
    };
    let (control, bytes) = invoke(["-c", "cat sibling"], root.path(), None);
    assert!(control.success);
    assert_eq!(bytes, b"denied\n");
    let (permitted, bytes) = invoke(["-c", "cat allowed"], root.path(), Some(boundary()));
    assert!(permitted.success, "{}", String::from_utf8_lossy(&permitted.stderr));
    assert_eq!(bytes, b"allowed\n");
    let (refused, bytes) = invoke(["-c", "cat sibling"], root.path(), Some(boundary()));
    assert!(!refused.success);
    assert!(bytes.is_empty());
    assert!(Boundary::new().read_file(root.path()).is_err());
}

#[test]
fn runtime_lookup_preserves_and_admits_the_winning_path_spelling() {
    let root = tempfile::tempdir().unwrap();
    let name = root.path().join("selected-shell");
    std::os::unix::fs::symlink("/bin/sh", &name).unwrap();
    let environment = OperatorEnvironment::admit([
        ("PATH".into(), root.path().as_os_str().to_owned()),
    ]).unwrap();
    let selected = environment.resolve(Path::new("selected-shell")).unwrap();
    assert_eq!(selected.declared(), name);
    assert_eq!(selected.canonical(), std::fs::canonicalize("/bin/sh").unwrap());
    let mut observed = Vec::new();
    let error = RuntimeAccess::discover(&environment, Path::new("selected-shell"), |path| {
        observed.push(path.to_owned());
        Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "source-spelled program"))
    }).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    assert_eq!(observed, [name]);
}

#[test]
fn native_runtime_authority_lists_files_not_executable_parent_directories() {
    let environment = OperatorEnvironment::capture().unwrap();
    let access = RuntimeAccess::discover(&environment, Path::new("/bin/sh"), |_| Ok(())).unwrap();
    assert!(access.files().contains(&std::fs::canonicalize("/bin/sh").unwrap()));
    assert!(access.files().iter().all(|path| path.is_file()));
    #[cfg(target_os = "linux")]
    assert!(access.directories().is_empty());
}
