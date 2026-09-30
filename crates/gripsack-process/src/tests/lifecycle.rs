use super::*;

#[test]
fn exit_and_signal_status_are_not_routing_policy() {
    let (o, lines) = peer("exit 3", b"", limits());
    assert!(matches!(o.reason, StopReason::Exited));
    assert_eq!(o.status.unwrap().code(), Some(3));
    assert!(lines.is_empty());
    let (o, _) = peer("kill -TERM $$", b"", limits());
    assert!(matches!(o.reason, StopReason::Exited));
    assert_eq!(o.status.unwrap().signal(), Some(libc::SIGTERM));
}

#[test]
fn no_response_is_a_normal_exit() {
    let (o, lines) = peer(":", b"", limits());
    assert!(matches!(o.reason, StopReason::Exited));
    assert!(o.status.unwrap().success());
    assert!(lines.is_empty());
}

#[test]
fn silent_process_with_closed_pipes_still_has_a_deadline() {
    let start = Instant::now();
    let (o, _) = peer("exec 1>&- 2>&-; exec sleep 60", b"", limits());
    assert!(matches!(o.reason, StopReason::Deadline), "{o:?}");
    assert_eq!(o.status.unwrap().signal(), Some(libc::SIGKILL));
    assert!(start.elapsed() < Duration::from_secs(8));
}

#[test]
fn inherited_pipes_are_killed_on_leader_exit_not_after_a_linger() {
    for redirection in ["", ">/dev/null", "2>/dev/null"] {
        let body = format!("sleep 60 {redirection} & printf 'done\\n'; exit 0");
        let start = Instant::now();
        let (o, lines) = peer(
            &body,
            b"",
            Limits {
                timeout: Duration::from_secs(20),
                ..limits()
            },
        );
        assert!(matches!(o.reason, StopReason::Exited), "{o:?}");
        assert_eq!(o.status.unwrap().code(), Some(0));
        assert_eq!(lines, [b"done".to_vec()]);
        assert!(start.elapsed() < Duration::from_secs(8));
    }
}

#[test]
fn callback_panic_kills_and_reaps_the_leader() {
    let mut pid = None;
    let start = Instant::now();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = run(
            &mut command("printf '%s\\n' $$; exec sleep 60"),
            b"",
            limits(),
            |line| {
                pid = Some(
                    std::str::from_utf8(line)
                        .unwrap()
                        .parse::<libc::pid_t>()
                        .unwrap(),
                );
                panic!("intentional callback panic");
            },
        );
    }));
    assert!(result.is_err());
    assert!(start.elapsed() < Duration::from_secs(8));
    // Observe, never signal: confirms guard reaped without risking PID reuse.
    let error = sys::observe(pid.unwrap()).unwrap_err();
    assert_eq!(error.raw_os_error(), Some(libc::ECHILD));
}

#[test]
fn completed_cleanup_cannot_erase_an_expired_operation() {
    let deadline = Instant::now() + Duration::from_secs(2);
    let child = command("exec sleep 60")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .unwrap();
    let pid = child.id() as libc::pid_t;
    let (mut guard, _pipes) = crate::lifecycle::Guard::new(child, deadline);
    // Establish successful signalling while the group is live. Darwin may
    // legitimately return EPERM for a zombie-only group; with an already
    // expired budget the classifier cannot prove that EPERM benign. That
    // earlier syscall error must remain observable, not be hidden by a timeout.
    guard.terminate().unwrap();
    let observation_deadline = Instant::now() + Duration::from_secs(2);
    while !guard.observe().unwrap() {
        assert!(Instant::now() < observation_deadline, "child did not exit");
        std::thread::yield_now();
    }
    // The original operation allowance expires after successful termination.
    // There are no pipes to drain; only otherwise-complete cleanup is tested.
    // Loop rather than one sleep: an early wake must never let the deadline
    // look unexpired to the guard.
    while Instant::now() < deadline {
        std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
    }
    let (status, error) = guard.finish(|_| Ok(true));
    assert_eq!(status.unwrap().signal(), Some(libc::SIGKILL));
    assert!(
        matches!(
            error.as_ref().map(io::Error::kind),
            Some(io::ErrorKind::TimedOut)
        ),
        "expired_cleanup_was_reported_successfully: {error:?}",
    );
    assert_eq!(
        sys::observe(pid).unwrap_err().raw_os_error(),
        Some(libc::ECHILD),
    );
}
