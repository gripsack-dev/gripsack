use super::*;
use std::os::unix::process::ExitStatusExt;

#[cfg(target_os = "linux")]
mod confinement;
mod lifecycle;
mod native;
mod pressure;

fn command(body: &str) -> Command {
    let mut command = Command::new("/bin/sh");
    command.arg("-c").arg(body);
    command
}

fn limits() -> Limits {
    Limits {
        timeout: Duration::from_secs(4),
        ..Limits::default()
    }
}

fn peer(body: &str, input: &[u8], limits: Limits) -> (Outcome, Vec<Vec<u8>>) {
    let mut lines = Vec::new();
    let outcome = run(&mut command(body), input, limits, |line| {
        lines.push(line.to_vec());
        Control::Continue
    })
    .unwrap();
    (outcome, lines)
}

#[test]
fn framing_and_input_eof() {
    let (o, lines) = peer("cat; printf '\n\nfinal'", b"alpha\nbeta\r\n", limits());
    assert!(matches!(o.reason, StopReason::Exited), "{o:?}");
    assert!(o.status.unwrap().success());
    assert_eq!(
        lines,
        [
            b"alpha".to_vec(),
            b"beta\r".to_vec(),
            vec![],
            vec![],
            b"final".to_vec()
        ]
    );
}

#[test]
fn empty_input_is_closed_immediately() {
    let (o, lines) = peer("cat; printf done", b"", limits());
    assert!(matches!(o.reason, StopReason::Exited));
    assert_eq!(lines, [b"done".to_vec()]);
}

#[test]
fn input_rejected_before_spawn() {
    let mut c = Command::new("/nonexistent/gripsack-test");
    let o = run(
        &mut c,
        b"xx",
        Limits {
            input_bytes: crate::InputByteLimit::new(1),
            ..limits()
        },
        |_| panic!(),
    )
    .unwrap();
    assert!(matches!(o.reason, StopReason::InputLimit));
    assert!(o.status.is_none());
}

#[test]
fn spawn_error_and_zero_budget() {
    let mut c = Command::new("/nonexistent/gripsack-test");
    assert_eq!(
        run(&mut c, b"", limits(), |_| Control::Continue)
            .unwrap_err()
            .kind(),
        io::ErrorKind::NotFound
    );
    let o = run(
        &mut c,
        b"",
        Limits {
            timeout: Duration::ZERO,
            ..limits()
        },
        |_| panic!(),
    )
    .unwrap();
    assert!(matches!(o.reason, StopReason::Deadline));
    assert!(o.status.is_none());
}

#[test]
fn expired_operation_rejects_spawn_without_overriding_input_failure() {
    let mut command = Command::new("/nonexistent/gripsack-test");
    let limits = Limits {
        operation_deadline: Some(Instant::now()),
        input_bytes: crate::InputByteLimit::new(1),
        ..limits()
    };
    let expired = run(&mut command, b"", limits, |_| panic!()).unwrap();
    assert!(matches!(expired.reason, StopReason::Deadline));
    assert!(expired.status.is_none());
    let oversized = run(&mut command, b"xx", limits, |_| panic!()).unwrap();
    assert!(matches!(oversized.reason, StopReason::InputLimit));
    assert!(oversized.status.is_none());
}

#[test]
fn operation_and_exchange_deadlines_each_bound_cleanup() {
    for (timeout, operation_budget) in [
        (Duration::from_secs(8), Duration::from_secs(1)),
        (Duration::from_secs(1), Duration::from_secs(8)),
    ] {
        let start = Instant::now();
        let (outcome, _) = peer(
            "exec sleep 60",
            b"",
            Limits {
                timeout,
                operation_deadline: Some(start + operation_budget),
                ..limits()
            },
        );
        assert!(
            matches!(outcome.reason, StopReason::Deadline),
            "{outcome:?}"
        );
        assert!(outcome.status.is_some(), "the owned leader must be reaped");
        assert!(start.elapsed() < Duration::from_secs(4));
    }
}

#[test]
fn tail_is_exact_and_can_be_disabled() {
    for cap in [0, 4, 100] {
        let (o, _) = peer(
            "printf 0123456789abc >&2",
            b"",
            Limits {
                retained_stderr_bytes: crate::RetainedStderrLimit::new(cap),
                ..limits()
            },
        );
        assert!(matches!(o.reason, StopReason::Exited), "{o:?}");
        let bytes = b"0123456789abc";
        assert_eq!(o.stderr, bytes[bytes.len().saturating_sub(cap)..]);
    }
}

#[test]
fn exact_line_limit_and_empty_lines() {
    let (o, lines) = peer(
        "printf 'abcd\nlast'",
        b"",
        Limits {
            line_bytes: crate::FrameByteLimit::new(4),
            ..limits()
        },
    );
    assert!(matches!(o.reason, StopReason::Exited));
    assert_eq!(lines, [b"abcd".to_vec(), b"last".to_vec()]);
    let (o, lines) = peer(
        "printf '\n\n'",
        b"",
        Limits {
            line_bytes: crate::FrameByteLimit::new(0),
            ..limits()
        },
    );
    assert!(matches!(o.reason, StopReason::Exited));
    assert_eq!(lines, [vec![], vec![]]);
}
