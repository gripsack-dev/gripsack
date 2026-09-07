use super::*;
use std::os::unix::process::ExitStatusExt;

mod lifecycle;
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
fn defaults_match_contract() {
    let l = Limits::default();
    assert_eq!(l.timeout, Duration::from_secs(600));
    assert_eq!(l.input_bytes, 4 * 1024 * 1024);
    assert_eq!(l.line_bytes, 1024 * 1024);
    assert_eq!(l.stdout_bytes, 16 * 1024 * 1024);
    assert_eq!(l.stderr_bytes, 16 * 1024 * 1024);
    assert_eq!(l.retained_stderr_bytes, 64 * 1024);
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
            input_bytes: 1,
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
fn tail_is_exact_and_can_be_disabled() {
    for cap in [0, 4, 100] {
        let (o, _) = peer(
            "printf 0123456789abc >&2",
            b"",
            Limits {
                retained_stderr_bytes: cap,
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
            line_bytes: 4,
            ..limits()
        },
    );
    assert!(matches!(o.reason, StopReason::Exited));
    assert_eq!(lines, [b"abcd".to_vec(), b"last".to_vec()]);
    let (o, lines) = peer(
        "printf '\n\n'",
        b"",
        Limits {
            line_bytes: 0,
            ..limits()
        },
    );
    assert!(matches!(o.reason, StopReason::Exited));
    assert_eq!(lines, [vec![], vec![]]);
}
