use super::*;

#[test]
fn simultaneous_input_and_output_pressure() {
    // More than any ordinary pipe capacity in each direction. The background
    // stderr writer must finish before the leader exits (otherwise killed).
    let body = "(dd if=/dev/zero bs=8192 count=64 >&2 2>/dev/null) & p=$!; cat; wait $p";
    let input = vec![b'x'; 512 * 1024];
    let (o, lines) = peer(body, &input, limits());
    assert!(matches!(o.reason, StopReason::Exited), "{o:?}");
    assert!(o.status.unwrap().success());
    assert_eq!(lines, [input]);
    assert!(
        o.stderr.len() == 64 * 1024 && o.stderr.iter().all(|byte| *byte == 0),
        "expected a full zero-byte stderr tail; received {} bytes",
        o.stderr.len()
    );
}

#[test]
fn blocked_input_does_not_block_response_or_deadline() {
    let mut seen = 0;
    let start = Instant::now();
    let o = run(
        &mut command("printf 'ready\n'; exec sleep 60"),
        &vec![b'x'; 512 * 1024],
        limits(),
        |line| {
            assert_eq!(line, b"ready");
            seen += 1;
            Control::Response
        },
    )
    .unwrap();
    assert_eq!(seen, 1);
    assert!(matches!(o.reason, StopReason::Deadline), "{o:?}");
    assert!(o.status.is_some());
    assert!(start.elapsed() < Duration::from_secs(8));
}

#[test]
fn huge_unterminated_line_is_stopped_incrementally() {
    let (o, lines) = peer(
        "exec dd if=/dev/zero bs=8192 count=1024 2>/dev/null",
        b"",
        Limits {
            line_bytes: 16384,
            ..limits()
        },
    );
    assert!(matches!(o.reason, StopReason::LineLimit), "{o:?}");
    assert!(lines.is_empty());
    assert!(o.status.is_some());
}

#[test]
fn stdout_and_stderr_floods_are_cumulative() {
    let (o, _) = peer(
        "exec yes x",
        b"",
        Limits {
            stdout_bytes: 32768,
            ..limits()
        },
    );
    assert!(matches!(o.reason, StopReason::StdoutLimit), "{o:?}");
    let (o, _) = peer(
        "exec yes diagnostic >&2",
        b"",
        Limits {
            stderr_bytes: 32768,
            retained_stderr_bytes: 1024,
            ..limits()
        },
    );
    assert!(matches!(o.reason, StopReason::StderrLimit), "{o:?}");
    assert_eq!(o.stderr.len(), 1024);
}

#[test]
fn response_does_not_disable_any_output_bound() {
    for (body, l, expected) in [
        (
            "printf 'ok\n'; exec yes x",
            Limits {
                stdout_bytes: 32768,
                ..limits()
            },
            0,
        ),
        (
            "printf 'ok\n'; exec yes x >&2",
            Limits {
                stderr_bytes: 32768,
                ..limits()
            },
            1,
        ),
        (
            "printf 'ok\n'; exec dd if=/dev/zero bs=8192 count=100 2>/dev/null",
            Limits {
                line_bytes: 16384,
                ..limits()
            },
            2,
        ),
    ] {
        let mut calls = 0;
        let o = run(&mut command(body), b"", l, |line| {
            calls += 1;
            assert_eq!(line, b"ok");
            Control::Response
        })
        .unwrap();
        assert_eq!(calls, 1);
        assert!(
            match expected {
                0 => matches!(o.reason, StopReason::StdoutLimit),
                1 => matches!(o.reason, StopReason::StderrLimit),
                _ => matches!(o.reason, StopReason::LineLimit),
            },
            "{o:?}"
        );
    }
}

#[test]
fn response_still_sends_input_and_preserves_exit_failure() {
    let mut calls = 0;
    let o = run(
        &mut command("printf 'ok\n'; cat >/dev/null; printf 'ignored\n'; exit 7"),
        &vec![b'x'; 512 * 1024],
        limits(),
        |_| {
            calls += 1;
            Control::Response
        },
    )
    .unwrap();
    assert_eq!(calls, 1);
    assert!(matches!(o.reason, StopReason::Exited), "{o:?}");
    assert_eq!(o.status.unwrap().code(), Some(7));
}

#[test]
fn early_stdin_close_is_not_an_io_error() {
    let (o, lines) = peer(
        "exec 0<&-; printf 'closed\n'; sleep 0.1",
        &vec![b'x'; 512 * 1024],
        limits(),
    );
    assert!(matches!(o.reason, StopReason::Exited), "{o:?}");
    assert_eq!(lines, [b"closed".to_vec()]);
}
