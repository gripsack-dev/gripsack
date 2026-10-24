use super::*;
use std::{
    ffi::{OsStr, OsString},
    fs::{self, File},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::ffi::OsStrExt,
    },
    path::Path,
};

// Process-control handlers and inherited stdin belong to the process, so each
// probe runs in an isolated test executable, not alongside parallel unit tests.
fn isolated(name: &str, probe: impl FnOnce()) {
    const PROBE: &str = "GRIPSACK_TASK_SUPERVISION_PROBE";
    if std::env::var(PROBE).as_deref() == Ok(name) {
        probe();
        return;
    }
    let test = format!("tests::task::{name}");
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", &test, "--test-threads=1", "--nocapture"])
        .env(PROBE, name);
    let result = run_raw(
        &mut command,
        b"",
        Limits {
            timeout: Duration::from_secs(20),
            ..limits()
        },
        |_| Control::Continue,
    )
    .unwrap();
    assert!(
        result.status.is_some_and(|status| status.success()),
        "{result:?}"
    );
}

struct Fixture {
    directory: tempfile::TempDir,
    environment: OperatorEnvironment,
    program: SelectedProgram,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("root"), b"retained consumer root").unwrap();
        let environment =
            OperatorEnvironment::admit([(OsString::from("PATH"), OsString::from("/usr/bin:/bin"))])
                .unwrap();
        let program = SelectedProgram::select(
            &environment,
            Path::new("/bin/sh"),
            None,
            Instant::now() + limits().timeout,
        )
        .unwrap();
        Self {
            directory,
            environment,
            program,
        }
    }

    fn invocation(&self, limits: Limits) -> Invocation<'_> {
        self.invocation_for(ProcessRole::Task, limits)
    }

    fn invocation_for(&self, role: ProcessRole, limits: Limits) -> Invocation<'_> {
        Invocation::admit(
            &self.environment,
            role,
            &self.program,
            self.directory.path(),
            limits,
        )
        .unwrap()
        .retain_leases(ProcessLeases {
            worker: None,
            retention: Some(File::open(self.directory.path().join("root")).unwrap()),
        })
        .unwrap()
    }
}

#[test]
fn task_closed_stdin_and_native_identity() {
    isolated("task_closed_stdin_and_native_identity", || {
        let fixture = Fixture::new();
        let invocation = fixture.invocation(limits());
        let root = invocation.leases.retention.as_ref().unwrap().as_raw_fd();
        let script = format!(
            "if read -r unexpected; then exit 90; fi; printf '%s\\000' \"$#\" \"$@\" \"$PATH\"; pwd; cat /dev/fd/{root}"
        );
        let arguments = [
            OsStr::new("-c"),
            OsStr::new(&script),
            OsStr::new("task"),
            OsStr::new(""),
            OsStr::new("a b\n'\"$"),
            OsStr::from_bytes(b"non-utf8-\xff"),
        ];
        let mut output = Vec::new();
        let result = invocation
            .run_task(&arguments, false, |bytes| {
                output.extend_from_slice(bytes);
                Control::Continue
            })
            .unwrap();
        assert!(result.success, "{:?}", result.receipt);
        let mut expected = b"3\0\0a b\n'\"$\0non-utf8-\xff\0/usr/bin:/bin\0".to_vec();
        expected.extend_from_slice(fixture.directory.path().as_os_str().as_bytes());
        expected.extend_from_slice(b"\nretained consumer root");
        assert_eq!(output, expected);
        assert!(result.receipt.deadline_millis > 0);
        assert_eq!(result.receipt.stdout_limit, limits().stdout_bytes.bytes());
    });
}

#[test]
fn task_inherited_stdin_deadline() {
    isolated("task_inherited_stdin_deadline", || {
        let fixture = Fixture::new();
        let mut pipes = [-1; 2];
        // Keep the write end open and silent: EOF cannot release the child.
        assert_eq!(
            unsafe { libc::pipe2(pipes.as_mut_ptr(), libc::O_CLOEXEC) },
            0
        );
        let reader = unsafe { File::from_raw_fd(pipes[0]) };
        let _writer = unsafe { File::from_raw_fd(pipes[1]) };
        assert_eq!(
            unsafe { libc::dup2(reader.as_raw_fd(), libc::STDIN_FILENO) },
            0
        );
        let start = Instant::now();
        let invocation = fixture.invocation(Limits {
            timeout: Duration::from_secs(10),
            operation_deadline: Some(start + Duration::from_millis(600)),
            ..limits()
        });
        let result = invocation
            .run_task(
                &[OsStr::new("-c"), OsStr::new("read -r never; exit 91")],
                true,
                |_| Control::Continue,
            )
            .unwrap();
        assert_eq!(result.receipt.disposition, ProcessDisposition::Deadline);
        assert_eq!(result.receipt.signal, Some(libc::SIGKILL));
        assert!(!result.success);
        assert!(start.elapsed() < Duration::from_secs(3));
    });
}

#[test]
fn task_cancellation_kills_descendants() {
    isolated("task_cancellation_kills_descendants", || {
        let fixture = Fixture::new();
        let invocation = fixture.invocation(limits());
        let script = "trap '' TERM; sh -c 'trap \"\" TERM; echo $$ > descendant; while :; do sleep 10; done' & while [ ! -s descendant ]; do :; done; printf ready; wait";
        let mut signalled = false;
        let result = invocation
            .run_task(&[OsStr::new("-c"), OsStr::new(script)], false, |_| {
                if !signalled {
                    signalled = true;
                    assert_eq!(unsafe { libc::kill(libc::getpid(), libc::SIGTERM) }, 0);
                }
                Control::Continue
            })
            .unwrap();
        assert!(signalled);
        assert_eq!(result.receipt.disposition, ProcessDisposition::Cancelled);
        assert_eq!(result.receipt.signal, Some(libc::SIGKILL));
        assert!(!result.success);
        let pid: i32 = fs::read_to_string(fixture.directory.path().join("descendant"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert_not_running(pid);
    });
}

#[test]
fn verification_cancellation_reaps_child_and_kills_descendants() {
    isolated(
        "verification_cancellation_reaps_child_and_kills_descendants",
        || {
            let fixture = Fixture::new();
            let invocation = fixture.invocation_for(ProcessRole::Verify, limits());
            let script = "trap '' INT TERM; echo $$ > child; sh -c 'trap \"\" INT TERM; echo $$ > descendant; while :; do sleep 10; done' & while [ ! -s descendant ]; do :; done; printf ready; wait";
            let mut signalled = false;
            let result = invocation
                .run_verification(&[OsStr::new("-c"), OsStr::new(script)], |_| {
                    if !signalled {
                        signalled = true;
                        assert_eq!(unsafe { libc::kill(libc::getpid(), libc::SIGINT) }, 0);
                    }
                    Control::Continue
                })
                .unwrap();
            assert!(signalled);
            assert_eq!(result.receipt.disposition, ProcessDisposition::Cancelled);
            assert_eq!(result.receipt.signal, Some(libc::SIGKILL));
            assert!(!result.success);
            let child: i32 = fs::read_to_string(fixture.directory.path().join("child"))
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            assert_eq!(unsafe { libc::kill(child, 0) }, -1);
            assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
            let descendant: i32 = fs::read_to_string(fixture.directory.path().join("descendant"))
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            assert_not_running(descendant);
        },
    );
}

#[test]
fn verification_requires_role_and_root() {
    let fixture = Fixture::new();
    for role in [
        ProcessRole::Hook,
        ProcessRole::Task,
        ProcessRole::Build,
        ProcessRole::Verify,
    ] {
        let invocation = Invocation::admit(
            &fixture.environment,
            role,
            &fixture.program,
            fixture.directory.path(),
            limits(),
        )
        .unwrap();
        let invocation = if matches!(role, ProcessRole::Task | ProcessRole::Build) {
            invocation
                .retain_leases(ProcessLeases {
                    worker: None,
                    retention: Some(File::open(fixture.directory.path().join("root")).unwrap()),
                })
                .unwrap()
        } else {
            invocation
        };
        let error = invocation
            .run_verification(&[OsStr::new("-c"), OsStr::new("touch forbidden")], |_| {
                Control::Continue
            })
            .err()
            .unwrap();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }
    assert!(!fixture.directory.path().join("forbidden").exists());
}

fn assert_not_running(pid: i32) {
    let end = Instant::now() + Duration::from_secs(2);
    loop {
        match fs::read_to_string(format!("/proc/{pid}/stat")) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return,
            Ok(stat)
                if stat
                    .rsplit_once(") ")
                    .is_some_and(|(_, rest)| rest.starts_with('Z')) =>
            {
                return;
            }
            result => assert!(Instant::now() < end, "descendant still running: {result:?}"),
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn task_caps_after_response() {
    isolated("task_caps_after_response", || {
        let fixture = Fixture::new();
        for (script, disposition) in [
            (
                "printf ok; while [ ! -e callback ]; do :; done; while :; do printf 123456789; done",
                ProcessDisposition::StdoutLimit,
            ),
            (
                "printf ok; while [ ! -e callback ]; do :; done; while :; do printf 123456789 >&2; done",
                ProcessDisposition::StderrLimit,
            ),
        ] {
            let invocation = fixture.invocation(Limits {
                stdout_bytes: StdoutByteLimit::new(64),
                stderr_bytes: StderrByteLimit::new(64),
                retained_stderr_bytes: RetainedStderrLimit::new(5),
                ..limits()
            });
            let mut callbacks = 0;
            let result = invocation
                .run_task(&[OsStr::new("-c"), OsStr::new(script)], false, |_| {
                    callbacks += 1;
                    fs::write(fixture.directory.path().join("callback"), b"responded").unwrap();
                    Control::Response
                })
                .unwrap();
            fs::remove_file(fixture.directory.path().join("callback")).unwrap();
            assert_eq!(result.receipt.disposition, disposition);
            assert!(!result.success);
            assert_eq!(callbacks, 1);
            assert!(result.stderr.len() <= 5);
            if disposition == ProcessDisposition::StderrLimit {
                assert!(result.stderr_truncated);
            }
        }
    });
}

#[test]
fn task_requires_role_and_root() {
    isolated("task_requires_role_and_root", || {
        let fixture = Fixture::new();
        for role in [ProcessRole::Hook, ProcessRole::Task, ProcessRole::Build] {
            let invocation = Invocation::admit(
                &fixture.environment,
                role,
                &fixture.program,
                fixture.directory.path(),
                limits(),
            )
            .unwrap();
            let invocation = if role == ProcessRole::Build {
                invocation
                    .retain_leases(ProcessLeases {
                        worker: None,
                        retention: Some(File::open(fixture.directory.path().join("root")).unwrap()),
                    })
                    .unwrap()
            } else {
                invocation
            };
            let error = invocation
                .run_task(
                    &[OsStr::new("-c"), OsStr::new("touch forbidden")],
                    false,
                    |_| Control::Continue,
                )
                .err()
                .unwrap();
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        }
        assert!(!fixture.directory.path().join("forbidden").exists());
    });
}

#[test]
fn task_unknown_cleanup_retains_root() {
    isolated("task_unknown_cleanup_retains_root", || {
        let fixture = Fixture::new();
        let invocation = fixture.invocation(Limits {
            timeout: Duration::from_millis(800),
            ..limits()
        });
        let root = invocation.leases.retention.as_ref().unwrap();
        assert_eq!(unsafe { libc::flock(root.as_raw_fd(), libc::LOCK_EX) }, 0);
        // setsid deliberately escapes the documented process-group boundary
        // while retaining stdout and the real root lease. Its fallback timeout
        // bounds test debris even if an assertion fails before explicit kill.
        let script = "setsid sh -c 'echo $$ > escaped; sleep 5' & while [ ! -s escaped ]; do :; done; exit 0";
        let result = invocation
            .run_task(&[OsStr::new("-c"), OsStr::new(script)], false, |_| {
                Control::Continue
            })
            .unwrap();
        let pid: i32 = fs::read_to_string(fixture.directory.path().join("escaped"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        drop(invocation);
        let contender = File::open(fixture.directory.path().join("root")).unwrap();
        let locked = unsafe { libc::flock(contender.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        let error = io::Error::last_os_error();
        // Kill the escaped group explicitly; production correctly cannot claim
        // to have cleaned a group it never owned.
        unsafe { libc::killpg(pid, libc::SIGKILL) };
        assert_eq!(
            result.receipt.disposition,
            ProcessDisposition::CleanupFailure
        );
        assert_eq!(
            result.receipt.cleanup_cause,
            Some(ProcessDisposition::Deadline)
        );
        assert_eq!(result.receipt.exit_code, Some(0));
        assert!(!result.success);
        assert_eq!(locked, -1, "escaped descendant lost its retention lease");
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert_not_running(pid);
    });
}
