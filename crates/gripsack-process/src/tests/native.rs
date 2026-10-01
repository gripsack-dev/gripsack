use super::*;
use std::{
    ffi::{OsStr, OsString},
    fs,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::PermissionsExt,
    },
    path::Path,
};

fn operator(values: &[(&str, &str)]) -> OperatorEnvironment {
    OperatorEnvironment::admit(
        values
            .iter()
            .map(|(key, value)| (OsString::from(key), OsString::from(value))),
    )
    .unwrap()
}

#[test]
fn selected_script_survives_source_replacement_and_stale_approval_refuses() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("hook");
    let original = b"#!/bin/sh\nprintf original > effect\n";
    fs::write(&path, original).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    let environment = operator(&[("PATH", "/usr/bin:/bin")]);
    let digest = Sha256Digest::of(original);
    let deadline = std::time::Instant::now() + limits().timeout;
    let selected = SelectedProgram::select(&environment, &path, Some(digest), deadline).unwrap();
    fs::write(&path, b"#!/bin/sh\nprintf replaced > effect\n").unwrap();
    let invocation = Invocation::admit(
        &environment,
        ProcessRole::Hook,
        &selected,
        temporary.path(),
        Limits {
            operation_deadline: Some(deadline),
            ..limits()
        },
    )
    .unwrap();
    let outcome = invocation
        .run(&[], NativeInput::Bytes(b""), None, |_| Control::Continue)
        .unwrap();
    assert!(outcome.success, "{:?}", outcome.receipt);
    assert_eq!(
        fs::read(temporary.path().join("effect")).unwrap(),
        b"original"
    );
    fs::remove_file(temporary.path().join("effect")).unwrap();
    assert!(SelectedProgram::select(&environment, &path, Some(digest), deadline).is_err());
    assert!(!temporary.path().join("effect").exists());
}

#[test]
fn native_child_cannot_read_ungranted_environment_or_inherited_descriptor() {
    let temporary = tempfile::tempdir().unwrap();
    let canary_path = temporary.path().join("descriptor-canary");
    fs::write(&canary_path, b"private descriptor bytes").unwrap();
    let original = fs::File::open(&canary_path).unwrap();
    // SAFETY: dup returns a new owned non-CLOEXEC descriptor. It is kept open
    // throughout invocation and transferred to exactly one File owner.
    let descriptor = unsafe { libc::dup(original.as_raw_fd()) };
    assert!(descriptor >= 0);
    let canary = unsafe { fs::File::from_raw_fd(descriptor) };
    let environment = operator(&[
        ("PATH", "/usr/bin:/bin"),
        ("DUMMY_SECRET", "must-not-inherit"),
        ("LD_PRELOAD", "/nonexistent/gripsack-canary"),
        ("BASH_ENV", "/nonexistent/gripsack-canary"),
    ]);
    let deadline = std::time::Instant::now() + limits().timeout;
    let selected =
        SelectedProgram::select(&environment, Path::new("/bin/sh"), None, deadline).unwrap();
    let invocation = Invocation::admit(
        &environment,
        ProcessRole::Hook,
        &selected,
        temporary.path(),
        Limits {
            operation_deadline: Some(deadline),
            ..limits()
        },
    )
    .unwrap();
    let script = format!(
        "set -eu\n[ -z \"${{DUMMY_SECRET+x}}\" ]\n[ -z \"${{LD_PRELOAD+x}}\" ]\n[ -z \"${{BASH_ENV+x}}\" ]\nif cat /dev/fd/{descriptor} > stolen 2>/dev/null; then exit 91; fi\nprintf safe > effect\n"
    );
    let outcome = invocation
        .run(
            &[OsStr::new("-s")],
            NativeInput::Script(&script),
            None,
            |_| Control::Continue,
        )
        .unwrap();
    assert!(outcome.success, "{:?}", outcome.receipt);
    assert_eq!(fs::read(temporary.path().join("effect")).unwrap(), b"safe");
    assert_ne!(
        fs::read(temporary.path().join("stolen")).unwrap(),
        b"private descriptor bytes"
    );
    assert_eq!(fs::read(&canary_path).unwrap(), b"private descriptor bytes");
    drop(canary);
}

#[test]
fn explicit_coordination_descriptor_survives_while_ambient_descriptor_closes() {
    let temporary = tempfile::tempdir().unwrap();
    let lease_path = temporary.path().join("lease");
    let ambient_path = temporary.path().join("ambient");
    fs::write(&lease_path, b"retained lease").unwrap();
    fs::write(&ambient_path, b"ungranted bytes").unwrap();
    let lease = fs::File::open(&lease_path).unwrap();
    let ambient = fs::File::open(&ambient_path).unwrap();
    let lease_fd = lease.as_raw_fd();
    let ambient_fd = ambient.as_raw_fd();
    let environment = operator(&[("PATH", "/usr/bin:/bin")]);
    let deadline = std::time::Instant::now() + limits().timeout;
    let selected =
        SelectedProgram::select(&environment, Path::new("/bin/sh"), None, deadline).unwrap();
    let invocation = Invocation::admit(
        &environment,
        ProcessRole::Build,
        &selected,
        temporary.path(),
        Limits {
            operation_deadline: Some(deadline),
            ..limits()
        },
    )
    .unwrap()
    .retain_leases(ProcessLeases {
        worker: Some(lease),
        retention: None,
    })
    .unwrap();
    let script = format!(
        "set -eu\ncat /dev/fd/{lease_fd} > retained\nif cat /dev/fd/{ambient_fd} > stolen 2>/dev/null; then exit 91; fi\n"
    );
    let outcome = invocation
        .run(
            &[OsStr::new("-s")],
            NativeInput::Script(&script),
            None,
            |_| Control::Continue,
        )
        .unwrap();
    assert!(outcome.success, "{:?}", outcome.receipt);
    assert_eq!(
        fs::read(temporary.path().join("retained")).unwrap(),
        b"retained lease"
    );
    assert_ne!(
        fs::read(temporary.path().join("stolen")).unwrap(),
        b"ungranted bytes"
    );
}

#[test]
fn raw_bytes_have_no_line_limit_but_keep_the_total_output_limit() {
    let mut raw = Vec::new();
    let outcome = run_raw(
        &mut command("printf 'a\\000b\\377\\nlast'"),
        b"",
        Limits {
            line_bytes: crate::FrameByteLimit::new(1),
            stdout_bytes: crate::StdoutByteLimit::new(10),
            ..limits()
        },
        |bytes| {
            raw.extend_from_slice(bytes);
            Control::Continue
        },
    )
    .unwrap();
    assert!(matches!(outcome.reason, StopReason::Exited));
    assert!(outcome.status.unwrap().success());
    assert_eq!(raw, b"a\0b\xff\nlast");
    let outcome = run_raw(
        &mut command("printf 0123456789; exec sleep 60"),
        b"",
        Limits {
            line_bytes: crate::FrameByteLimit::new(1),
            stdout_bytes: crate::StdoutByteLimit::new(9),
            ..limits()
        },
        |_| Control::Continue,
    )
    .unwrap();
    assert!(matches!(outcome.reason, StopReason::StdoutLimit));
    assert!(outcome.status.is_some());
}

#[test]
fn spawn_failure_keeps_the_kernel_error_and_does_not_report_success() {
    let temporary = tempfile::tempdir().unwrap();
    let executable = temporary.path().join("invalid-image");
    fs::write(&executable, b"not an executable image\n").unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let environment = operator(&[("PATH", "/usr/bin:/bin")]);
    let deadline = std::time::Instant::now() + limits().timeout;
    let selected = SelectedProgram::select(&environment, &executable, None, deadline).unwrap();
    let invocation = Invocation::admit(
        &environment,
        ProcessRole::Hook,
        &selected,
        temporary.path(),
        Limits {
            operation_deadline: Some(deadline),
            ..limits()
        },
    )
    .unwrap();
    let outcome = invocation
        .run(&[], NativeInput::Bytes(b""), None, |_| Control::Continue)
        .unwrap();
    assert!(!outcome.success);
    assert_eq!(
        outcome.receipt.disposition,
        ProcessDisposition::SpawnFailure
    );
    assert_eq!(outcome.receipt.error.unwrap().os_code, Some(libc::ENOEXEC));
    assert_eq!(outcome.receipt.exit_code, None);
}

#[test]
fn task_overlay_reaches_child_with_search_prefix_precedence() {
    let temporary = tempfile::tempdir().unwrap();
    let effect = temporary.path().join("effect");
    let environment = operator(&[
        ("PATH", "/usr/bin:/bin"),
        ("LANG", "operator"),
        ("HOME", temporary.path().to_str().unwrap()),
    ]);
    let overlay = EnvironmentOverlay::admit(
        [
            (OsString::from("GREETING"), OsString::from("from-repo")),
            (OsString::from("LANG"), OsString::from("declared")),
        ],
        [Path::new("/pkg/bin").to_path_buf()],
        [],
    )
    .unwrap();
    let deadline = std::time::Instant::now() + limits().timeout;
    let selected =
        SelectedProgram::select(&environment, Path::new("/bin/sh"), None, deadline).unwrap();
    let invocation = Invocation::admit(
        &environment,
        ProcessRole::Task,
        &selected,
        temporary.path(),
        Limits {
            operation_deadline: Some(deadline),
            ..limits()
        },
    )
    .unwrap()
    .with_overlay(overlay);
    let body = "env > effect";
    let outcome = invocation
        .run(
            &[OsStr::new("-c"), OsStr::new(body)],
            NativeInput::Bytes(b""),
            None,
            |_| Control::Continue,
        )
        .unwrap();
    assert!(outcome.success, "{:?}", outcome.receipt);
    let environment = fs::read_to_string(effect).unwrap();
    assert!(environment.contains("GREETING=from-repo\n"), "{environment}");
    assert!(environment.contains("LANG=declared\n"), "{environment}");
    assert!(
        environment.contains("PATH=/pkg/bin:/usr/bin:/bin\n"),
        "{environment}"
    );
}

#[test]
fn interactive_is_restricted_to_the_task_role() {
    let temporary = tempfile::tempdir().unwrap();
    let environment = operator(&[("PATH", "/usr/bin:/bin")]);
    let deadline = std::time::Instant::now() + limits().timeout;
    let selected =
        SelectedProgram::select(&environment, Path::new("/bin/sh"), None, deadline).unwrap();
    let invocation = Invocation::admit(
        &environment,
        ProcessRole::Hook,
        &selected,
        temporary.path(),
        Limits {
            operation_deadline: Some(deadline),
            ..limits()
        },
    )
    .unwrap();
    assert_eq!(
        invocation.run_interactive(&[]).map(|_| ()).unwrap_err().kind(),
        std::io::ErrorKind::InvalidInput
    );
}

#[test]
fn interactive_preserves_exact_argv_and_reports_exit_and_signal() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("probe");
    fs::write(
        &path,
        "#!/bin/sh\n{\nprintf '%s\\n' \"$#\"\nfor arg in \"$@\"; do printf '<%s>\\n' \"$arg\"; done\npwd\n} > effect\nexit \"${3:-0}\"\n",
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    let environment = operator(&[("PATH", "/usr/bin:/bin")]);
    let deadline = std::time::Instant::now() + limits().timeout;
    let selected = SelectedProgram::select(&environment, &path, None, deadline).unwrap();
    let admit = || {
        Invocation::admit(
            &environment,
            ProcessRole::Task,
            &selected,
            temporary.path(),
            Limits {
                operation_deadline: Some(deadline),
                ..limits()
            },
        )
        .unwrap()
    };
    let outcome = admit()
        .run_interactive(&[OsStr::new(""), OsStr::new("a b"), OsStr::new("7")])
        .unwrap();
    assert!(!outcome.success);
    assert_eq!(outcome.receipt.exit_code, Some(7));
    assert_eq!(outcome.receipt.signal, None);
    assert_eq!(outcome.receipt.disposition, ProcessDisposition::Exited);
    let recorded = fs::read_to_string(temporary.path().join("effect")).unwrap();
    let expected = format!("3\n<>\n<a b>\n<7>\n{}\n", temporary.path().canonicalize().unwrap().display());
    assert_eq!(recorded, expected);

    fs::write(&path, "#!/bin/sh\nkill -TERM $$\n").unwrap();
    // Selection binds bytes: the suicide script needs its own selection.
    let selected = SelectedProgram::select(&environment, &path, None, deadline).unwrap();
    let outcome = Invocation::admit(
        &environment,
        ProcessRole::Task,
        &selected,
        temporary.path(),
        Limits {
            operation_deadline: Some(deadline),
            ..limits()
        },
    )
    .unwrap()
    .run_interactive(&[])
    .unwrap();
    assert!(!outcome.success);
    assert_eq!(outcome.receipt.exit_code, None);
    assert_eq!(outcome.receipt.signal, Some(libc::SIGTERM));
    assert_eq!(outcome.receipt.disposition, ProcessDisposition::Exited);
}

#[test]
fn consumer_role_admits_retention_lease_but_hook_does_not() {
    let temporary = tempfile::tempdir().unwrap();
    let lease_path = temporary.path().join("lease");
    fs::write(&lease_path, b"retained root").unwrap();
    let environment = operator(&[("PATH", "/usr/bin:/bin")]);
    let deadline = std::time::Instant::now() + limits().timeout;
    let selected =
        SelectedProgram::select(&environment, Path::new("/bin/sh"), None, deadline).unwrap();
    let admit = |role| {
        Invocation::admit(
            &environment,
            role,
            &selected,
            temporary.path(),
            Limits {
                operation_deadline: Some(deadline),
                ..limits()
            },
        )
        .unwrap()
    };
    let leases = || ProcessLeases {
        worker: None,
        retention: Some(fs::File::open(&lease_path).unwrap()),
    };
    assert!(admit(ProcessRole::Task).retain_leases(leases()).is_ok());
    assert!(admit(ProcessRole::Hook).retain_leases(leases()).is_err());
}

#[test]
fn retained_authority_survives_closed_standard_descriptors() {
    isolated_native_probe(
        "tests::native::retained_authority_survives_closed_standard_descriptors",
        NativeProbe::ClosedStdio,
    );
}

#[cfg(target_os = "linux")]
#[test]
fn legacy_memfd_flag_support_keeps_sealed_execution() {
    isolated_native_probe(
        "tests::native::legacy_memfd_flag_support_keeps_sealed_execution",
        NativeProbe::LegacyMemfd,
    );
}

#[cfg(target_os = "linux")]
#[test]
fn legacy_descriptor_sweep_hides_inherited_canary() {
    isolated_native_probe(
        "tests::native::legacy_descriptor_sweep_hides_inherited_canary",
        NativeProbe::LegacyDescriptors,
    );
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NativeProbe {
    ClosedStdio,
    #[cfg(target_os = "linux")]
    LegacyMemfd,
    #[cfg(target_os = "linux")]
    LegacyDescriptors,
}

fn isolated_native_probe(test: &str, probe: NativeProbe) {
    const CHILD: &str = "GRIPSACK_NATIVE_TEST_CHILD_DIRECTORY";
    if let Some(directory) = std::env::var_os(CHILD) {
        let result = (|| -> io::Result<()> {
            let canary: Option<fs::File> = match probe {
                NativeProbe::ClosedStdio => {
                    // Only this re-executed test process loses its stdio.
                    unsafe {
                        libc::close(0);
                        libc::close(1);
                        libc::close(2);
                    }
                    None
                }
                #[cfg(target_os = "linux")]
                NativeProbe::LegacyMemfd => {
                    inject_unknown_exec_flag()?;
                    None
                }
                #[cfg(target_os = "linux")]
                NativeProbe::LegacyDescriptors => {
                    inject_missing_close_range()?;
                    let path = Path::new(&directory).join("descriptor-canary");
                    fs::write(&path, b"private inherited bytes")?;
                    let file = fs::File::open(path)?;
                    let descriptor = unsafe { libc::dup(file.as_raw_fd()) };
                    if descriptor < 0 {
                        return Err(io::Error::last_os_error());
                    }
                    Some(unsafe { fs::File::from_raw_fd(descriptor) })
                }
            };
            let script = match &canary {
                Some(file) => std::borrow::Cow::Owned(format!(
                    "if cat /dev/fd/{} > stolen 2>/dev/null; then exit 91; fi; printf bound > effect",
                    file.as_raw_fd()
                )),
                None => std::borrow::Cow::Borrowed("printf bound > effect"),
            };
            let environment = operator(&[("PATH", "/usr/bin:/bin")]);
            let deadline = std::time::Instant::now() + limits().timeout;
            let selected =
                SelectedProgram::select(&environment, Path::new("/bin/sh"), None, deadline)?;
            let invocation = Invocation::admit(
                &environment,
                ProcessRole::Hook,
                &selected,
                Path::new(&directory),
                Limits {
                    operation_deadline: Some(deadline),
                    ..limits()
                },
            )?;
            let result = invocation.run(
                &[OsStr::new("-c"), OsStr::new(script.as_ref())],
                NativeInput::Bytes(b""),
                None,
                |_| Control::Continue,
            )?;
            if !result.success
                || (probe != NativeProbe::ClosedStdio
                    && result.receipt.byte_binding != ByteBinding::ExecutableHandle)
            {
                return Err(io::Error::other(format!(
                    "native authority probe failed: {:?}",
                    result.receipt
                )));
            }
            Ok(())
        })();
        std::process::exit(if result.is_ok() { 0 } else { 1 });
    }
    let temporary = tempfile::tempdir().unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap());
    child
        .args(["--exact", test, "--test-threads=1", "--nocapture"])
        .env(CHILD, temporary.path());
    let outcome = run(
        &mut child,
        b"",
        Limits {
            timeout: Duration::from_secs(20),
            ..limits()
        },
        |_| Control::Continue,
    )
    .unwrap();
    assert!(outcome.status.unwrap().success(), "{outcome:?}");
    assert_eq!(fs::read(temporary.path().join("effect")).unwrap(), b"bound");
}

#[cfg(target_os = "linux")]
fn inject_unknown_exec_flag() -> io::Result<()> {
    // A software fault at the real syscall boundary, not an older-kernel claim:
    // return EINVAL only for memfd_create(... MFD_EXEC), as pre-flag kernels do.
    let flags_offset = std::mem::offset_of!(libc::seccomp_data, args) + std::mem::size_of::<u64>();
    let mut instructions = [
        libc::sock_filter {
            code: (libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16,
            jt: 0,
            jf: 0,
            k: 0,
        },
        libc::sock_filter {
            code: (libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K) as u16,
            jt: 0,
            jf: 3,
            k: libc::SYS_memfd_create as u32,
        },
        libc::sock_filter {
            code: (libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16,
            jt: 0,
            jf: 0,
            k: flags_offset as u32,
        },
        libc::sock_filter {
            code: (libc::BPF_JMP | libc::BPF_JSET | libc::BPF_K) as u16,
            jt: 0,
            jf: 1,
            k: libc::MFD_EXEC,
        },
        libc::sock_filter {
            code: (libc::BPF_RET | libc::BPF_K) as u16,
            jt: 0,
            jf: 0,
            k: libc::SECCOMP_RET_ERRNO | libc::EINVAL as u32,
        },
        libc::sock_filter {
            code: (libc::BPF_RET | libc::BPF_K) as u16,
            jt: 0,
            jf: 0,
            k: libc::SECCOMP_RET_ALLOW,
        },
    ];
    install_filter(&mut instructions)
}

#[cfg(target_os = "linux")]
fn inject_missing_close_range() -> io::Result<()> {
    let mut instructions = [
        libc::sock_filter {
            code: (libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16,
            jt: 0,
            jf: 0,
            k: 0,
        },
        libc::sock_filter {
            code: (libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K) as u16,
            jt: 0,
            jf: 1,
            k: libc::SYS_close_range as u32,
        },
        libc::sock_filter {
            code: (libc::BPF_RET | libc::BPF_K) as u16,
            jt: 0,
            jf: 0,
            k: libc::SECCOMP_RET_ERRNO | libc::ENOSYS as u32,
        },
        libc::sock_filter {
            code: (libc::BPF_RET | libc::BPF_K) as u16,
            jt: 0,
            jf: 0,
            k: libc::SECCOMP_RET_ALLOW,
        },
    ];
    install_filter(&mut instructions)
}

#[cfg(target_os = "linux")]
fn install_filter(instructions: &mut [libc::sock_filter]) -> io::Result<()> {
    let program = libc::sock_fprog {
        len: u16::try_from(instructions.len()).map_err(io::Error::other)?,
        filter: instructions.as_mut_ptr(),
    };
    // SAFETY: the filter is a bounded local instruction array; the kernel copies
    // it before returning. No-new-privileges/filter affect this child only.
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } < 0
        || unsafe { libc::prctl(libc::PR_SET_SECCOMP, libc::SECCOMP_MODE_FILTER, &program) } < 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
