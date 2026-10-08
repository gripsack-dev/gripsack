//! Run these on native GNU userspace, including RHEL8's backported 2.28
//! loader. The libc version does not select or bypass the capability test.
use super::*;
use std::{ffi::OsStr, fs, os::unix::fs::PermissionsExt, path::Path, sync::Arc};

fn platform_loader() -> &'static Path {
    #[cfg(target_arch = "x86_64")]
    let path = "/lib64/ld-linux-x86-64.so.2";
    #[cfg(target_arch = "aarch64")]
    let path = "/lib/ld-linux-aarch64.so.1";
    Path::new(path)
}

#[test]
fn sealed_gnu_loader_executes_original_bytes_and_preserves_argv_zero() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("selected-shell");
    let bytes = fs::read("/bin/sh").unwrap();
    fs::write(&path, &bytes).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    let environment = OperatorEnvironment::admit([
        ("PATH".into(), "/usr/bin:/bin".into()),
        ("LD_PRELOAD".into(), "/nonexistent/preload.so".into()),
        ("LD_AUDIT".into(), "/nonexistent/audit.so".into()),
        ("GLIBC_TUNABLES".into(), "glibc.cpu.hwcaps=hostile".into()),
    ])
    .unwrap();
    let deadline = Instant::now() + limits().timeout;
    let loader = Arc::new(SelectedGnuLoader::select(platform_loader(), deadline).unwrap());
    let selected = SelectedProgram::select(&environment, &path, Some(Sha256Digest::of(&bytes)), deadline)
        .unwrap().with_gnu_loader(loader, OsStr::new("/lib64:/usr/lib64:/lib/x86_64-linux-gnu:/usr/lib/x86_64-linux-gnu:/lib/aarch64-linux-gnu:/usr/lib/aarch64-linux-gnu")).unwrap();
    assert_eq!(
        selected.identity().executable_sha256,
        Sha256Digest::of(&bytes)
    );
    assert_eq!(
        selected.identity().loader_sha256,
        Some(Sha256Digest::of(&fs::read(platform_loader()).unwrap()))
    );
    // Neither the selected program nor its argv0 changes when its pathname is
    // replaced after admission. There is no unsealed pathname fallback.
    fs::write(&path, b"not the selected executable anymore").unwrap();
    let invocation = Invocation::admit(
        &environment,
        ProcessRole::Task,
        &selected,
        directory.path(),
        Limits {
            operation_deadline: Some(deadline),
            ..limits()
        },
    )
    .unwrap();
    let mut output = Vec::new();
    let result = invocation.run(&[OsStr::new("-c"), OsStr::new(
        "printf '%s' \"$0\"; test -z \"${LD_PRELOAD+x}${LD_AUDIT+x}${GLIBC_TUNABLES+x}\""
    )], NativeInput::Bytes(b""), None, |bytes| {
        output.extend_from_slice(bytes);
        Control::Continue
    }).unwrap();
    assert!(result.success, "{:?}: {:?}", result.receipt, result.stderr);
    assert_eq!(output, path.as_os_str().as_encoded_bytes());
    for key in ["LD_PRELOAD", "LD_AUDIT", "LD_HWCAP_MASK", "GLIBC_TUNABLES"] {
        let overlay = EnvironmentOverlay::admit(
            [(key.into(), "/nonexistent/declared-loader-input".into())],
            [],
            [],
        )
        .unwrap();
        let invocation = Invocation::admit(
            &environment,
            ProcessRole::Task,
            &selected,
            directory.path(),
            limits(),
        )
        .unwrap()
        .with_overlay(overlay);
        let error = match invocation.run(
            &[OsStr::new("-c"), OsStr::new("printf effect > unadmitted")],
            NativeInput::Bytes(b""),
            None,
            |_| Control::Continue,
        ) {
            Ok(_) => panic!("declared loader input reached execution: {key}"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(!directory.path().join("unadmitted").exists());
    }
}

#[test]
fn missing_real_loader_control_is_refused_even_without_a_version_check() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("loader-without-control");
    let original = fs::read(platform_loader()).unwrap();
    for control in [
        "--argv0",
        "--inhibit-cache",
        "--inhibit-rpath",
        "--library-path",
        "--glibc-hwcaps-mask",
    ] {
        let needle = control.as_bytes();
        let mut bytes = original.clone();
        let mut changed = 0;
        for offset in 0..=bytes.len().saturating_sub(needle.len()) {
            if &bytes[offset..offset + needle.len()] == needle {
                // Same-size semantic negatives in the real loader's option
                // and help strings. This remains an executable ELF loader.
                bytes[offset + needle.len() - 1] = b'!';
                changed += 1;
            }
        }
        assert!(changed > 0, "fixture loader lacks control {control}");
        fs::write(&path, bytes).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        let error = match SelectedGnuLoader::select(&path, Instant::now() + limits().timeout) {
            Ok(_) => panic!("loader missing {control} capability was admitted"),
            Err(error) => error,
        };
        assert_eq!(
            error.kind(),
            io::ErrorKind::Unsupported,
            "{control}: {error}"
        );
        assert!(error.to_string().contains(control));
    }
}
