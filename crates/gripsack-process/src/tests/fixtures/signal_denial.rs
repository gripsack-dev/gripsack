//! Real credential/capability boundaries, launched by check_process_bounds.py.
use gripsack_process::{Control, Limits, StopReason};
use std::os::unix::process::CommandExt;
use std::process::Command;
use std::time::Duration;

fn main() {
    let live = std::env::args().any(|arg| arg == "--live-credential-change");
    // SAFETY: geteuid has no arguments or memory preconditions.
    assert_eq!(unsafe { libc::geteuid() }, 0, "fixture requires isolated root");
    if !live {
        let status = std::fs::read_to_string("/proc/self/status").unwrap();
        let capabilities = status
            .lines()
            .find_map(|line| line.strip_prefix("CapEff:\t"))
            .unwrap();
        let effective = u64::from_str_radix(capabilities, 16).unwrap();
        assert_eq!(effective & (1 << 5), 0, "fixture requires CAP_KILL dropped");
    }
    let mut child = Command::new("/bin/sh");
    child
        .args(["-c", if live { "printf '%s\n' $$; sleep 8; exit 0" } else { "exit 0" }])
        .env_clear()
        .current_dir("/tmp")
        .uid(65534)
        .gid(65534);
    let mut pid = None;
    let outcome = gripsack_process::run(
        &mut child,
        b"",
        Limits {
            timeout: Duration::from_secs(if live { 5 } else { 2 }),
            ..Limits::default()
        },
        |line| {
            assert!(live);
            pid = Some(std::str::from_utf8(line).unwrap().parse::<libc::pid_t>().unwrap());
            // SAFETY: this single-threaded fixture runs in its own process.
            // Dropping all parent UID authority cannot affect the runner or
            // change wait ownership; the child retains a different UID.
            assert_eq!(unsafe { libc::setuid(65533) }, 0, "cannot drop fixture credentials");
            Control::Response
        },
    )
    .unwrap();
    println!("SIGNAL_DENIAL_OUTCOME={outcome:?}");
    if live {
        // The supervisor has returned a bounded cleanup failure and relinquished
        // its guard. Reap this finite first-party helper afterwards, not during
        // supervision, so the fixture does not leave a child behind.
        let mut status = 0;
        let pid = pid.expect("credential fixture did not reach its ready barrier");
        // SAFETY: exact own child PID, valid output pointer, no competing reaper.
        assert_eq!(unsafe { libc::waitpid(pid, &mut status, 0) }, pid);
        assert_eq!(status, 0);
        assert!(outcome.status.is_none());
    } else {
        assert_eq!(outcome.status.unwrap().code(), Some(0));
    }
    match &outcome.reason {
        StopReason::Cleanup { cause, error } => {
            assert_eq!(
                error.kind(),
                std::io::ErrorKind::PermissionDenied,
                "denied_group_signal_was_reported_successfully",
            );
            assert_eq!(error.raw_os_error(), Some(libc::EPERM), "cleanup_errno_was_lost");
            if live {
                assert!(matches!(cause.as_ref(), StopReason::Deadline), "first_deadline_was_lost");
            } else {
                assert!(
                    matches!(cause.as_ref(), StopReason::Io(error) if error.raw_os_error() == Some(libc::EPERM)),
                    "first_signal_denial_was_lost",
                );
            }
        }
        _ => panic!("denied_group_signal_was_reported_successfully: {outcome:?}"),
    }
    println!("SIGNAL_DENIAL_PROPERTY=passed live={live}");
}
