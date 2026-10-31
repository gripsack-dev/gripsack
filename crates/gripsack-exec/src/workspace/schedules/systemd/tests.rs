use super::*;

const JOB: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const REVISION: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn identities() -> (JobId, JobRevision) {
    (
        JobId::parse(JOB).unwrap(),
        JobRevision::parse(REVISION).unwrap(),
    )
}

fn daily() -> WorkspaceCalendar {
    WorkspaceCalendar::Daily {
        time: "09:07".into(),
    }
}

// Hand-authored wire fixtures, not snapshots obtained from render().
fn fixture() -> Units {
    Units {
        service: concat!(
            "[Unit]\nDescription=Gripsack scheduled task\n\n[Service]\nType=oneshot\n",
            "ExecStart=\":/retained/grip\" \"jobs\" \"--home\" \"/store/home\" \"dispatch\" ",
            "\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" ",
            "\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\"\n",
            "Restart=no\nRemainAfterExit=no\nStandardInput=null\n"
        ).into(),
        timer: concat!(
            "[Unit]\nDescription=Gripsack local calendar\n\n[Timer]\n",
            "OnCalendar=*-*-* 09:07:00\nPersistent=false\n",
            "Unit=gripsack-job-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.service\n",
            "\n[Install]\nWantedBy=timers.target\n"
        ).into(),
    }
}

fn validate_fixture(units: &Units) -> Result<(), ExecError> {
    let (job, revision) = identities();
    validate(
        units,
        &job,
        &revision,
        Path::new("/retained/grip"),
        Path::new("/store/home"),
        &daily(),
    )
}

#[test]
fn daily_matches_independent_fixture_and_decodes_actual_bytes() {
    let (job, revision) = identities();
    let expected = fixture();
    validate_fixture(&expected).unwrap();
    assert_eq!(
        render(
            &job,
            &revision,
            Path::new("/retained/grip"),
            Path::new("/store/home"),
            &daily()
        )
        .unwrap(),
        expected
    );
    assert_eq!(
        unit_names(&job),
        (
            format!("gripsack-job-{JOB}.service"),
            format!("gripsack-job-{JOB}.timer"),
        )
    );
}

#[test]
fn every_weekday_has_an_independent_calendar_fixture() {
    let (job, revision) = identities();
    for (weekday, text) in [
        (Weekday::Mon, "Mon"),
        (Weekday::Tue, "Tue"),
        (Weekday::Wed, "Wed"),
        (Weekday::Thu, "Thu"),
        (Weekday::Fri, "Fri"),
        (Weekday::Sat, "Sat"),
        (Weekday::Sun, "Sun"),
    ] {
        let calendar = WorkspaceCalendar::Weekly {
            weekday,
            time: "09:07".into(),
        };
        let mut expected = fixture();
        expected.timer = expected
            .timer
            .replace("OnCalendar=*-*-*", &format!("OnCalendar={text} *-*-*"));
        validate(
            &expected,
            &job,
            &revision,
            Path::new("/retained/grip"),
            Path::new("/store/home"),
            &calendar,
        )
        .unwrap();
        assert_eq!(
            render(
                &job,
                &revision,
                Path::new("/retained/grip"),
                Path::new("/store/home"),
                &calendar
            )
            .unwrap(),
            expected
        );
    }
}

#[test]
fn quoted_paths_preserve_specifiers_dollars_spaces_backslashes_and_unicode() {
    let (job, revision) = identities();
    let runner = Path::new("/retained/%n $HOME ${USER} ; 雪 grip");
    let home = Path::new("/store/%h $HOME ${X} 'quote'\" slash\\ café");
    let units = render(&job, &revision, runner, home, &daily()).unwrap();
    let command = units
        .service
        .lines()
        .find_map(|line| line.strip_prefix("ExecStart="))
        .unwrap();
    assert!(command.starts_with(r#"":/retained/%%n $HOME ${USER} ; 雪 grip" "jobs" "--home" "/store/%%h $HOME ${X} 'quote'\" slash\\ café""#));
    assert_eq!(
        decode_command(command).unwrap(),
        [
            runner.to_str().unwrap(),
            "jobs",
            "--home",
            home.to_str().unwrap(),
            "dispatch",
            JOB,
            REVISION,
        ]
    );
    validate(&units, &job, &revision, runner, home, &daily()).unwrap();
}

#[test]
fn actual_byte_decoder_rejects_command_policy_and_calendar_mutants() {
    for (before, after) in [
        ("/retained/grip", "/other/grip"),
        ("/store/home", "/other/home"),
        ("\"dispatch\"", "\"run\""),
        (REVISION, JOB),
        ("Restart=no", "Restart=always"),
        ("RemainAfterExit=no", "RemainAfterExit=yes"),
        ("StandardInput=null", "StandardInput=tty"),
        ("Type=oneshot", "Type=simple"),
        ("ExecStart=\":/", "ExecStart=\"/"),
        ("Restart=no", "Restart=no\nUser=root"),
        ("Restart=no", "Restart=no\nExecStart=/bin/false"),
    ] {
        let mut units = fixture();
        units.service = units.service.replace(before, after);
        assert!(
            validate_fixture(&units).is_err(),
            "accepted service mutant {after:?}"
        );
    }
    for (before, after) in [
        ("09:07:00", "09:08:00"),
        ("*-*-*", "Mon *-*-*"),
        ("Persistent=false", "Persistent=true"),
        ("timers.target", "multi-user.target"),
        (".service", ".timer"),
        ("09:07:00", "09:07:00 UTC"),
        ("Persistent=false", "Persistent=false\nOnBootSec=1"),
    ] {
        let mut units = fixture();
        units.timer = units.timer.replace(before, after);
        assert!(
            validate_fixture(&units).is_err(),
            "accepted timer mutant {after:?}"
        );
    }
}

#[test]
fn unescaped_specifiers_and_ambiguous_command_syntax_reject() {
    for command in [
        r#"":/bin/%n""#,
        r#"":/bin/a\x20b""#,
        r#"":/bin/a"; /bin/false"#,
        r#"":/bin/a" "unterminated"#,
        r#"+":/bin/a""#,
        r#""/bin/$HOME""#,
    ] {
        assert!(decode_command(command).is_err(), "accepted {command:?}");
    }
    assert_eq!(
        decode_command(r#"":/bin/%%n" "" "$X""#).unwrap(),
        ["/bin/%n", "", "$X"]
    );
}

#[test]
fn controls_invalid_utf8_relative_paths_and_oversized_units_reject() {
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt};
    let (job, revision) = identities();
    for control in ['\0', '\n', '\r', '\t', '\u{7f}', '\u{85}'] {
        let path = format!("/retained/{control}grip");
        assert!(
            render(
                &job,
                &revision,
                Path::new(&path),
                Path::new("/home"),
                &daily()
            )
            .is_err()
        );
        assert!(
            render(
                &job,
                &revision,
                Path::new("/grip"),
                Path::new(&path),
                &daily()
            )
            .is_err()
        );
        let mut units = fixture();
        units.service = units.service.replace("/retained/grip", &path);
        assert!(validate_fixture(&units).is_err());
    }
    for path in [
        Path::new("relative"),
        Path::new(OsStr::from_bytes(b"/non-utf8-\xff")),
    ] {
        assert!(render(&job, &revision, path, Path::new("/home"), &daily()).is_err());
    }
    for path in [
        "/quoted'grip",
        "/quoted\"grip",
        "/back\\slash",
        "/trailing/",
        "/a//grip",
        "/a/./grip",
        "/a/../grip",
    ] {
        assert!(
            render(
                &job,
                &revision,
                Path::new(path),
                Path::new("/home"),
                &daily()
            )
            .is_err()
        );
    }
    let mut units = fixture();
    units.service.push_str(&"x".repeat(MAX_UNIT_BYTES));
    assert!(validate_fixture(&units).is_err());
    let oversized = format!("/{}", "x".repeat(MAX_UNIT_BYTES));
    assert!(
        render(
            &job,
            &revision,
            Path::new(&oversized),
            Path::new("/home"),
            &daily()
        )
        .is_err()
    );
}

#[test]
fn calendar_admission_rejects_nonlocal_and_invalid_times() {
    let (job, revision) = identities();
    for time in [
        "",
        "9:07",
        "24:00",
        "09:60",
        "09:07:00",
        "09:07 UTC",
        "09:07\nPersistent=true",
        "é:07",
    ] {
        let trigger = WorkspaceCalendar::Daily { time: time.into() };
        assert!(
            render(
                &job,
                &revision,
                Path::new("/grip"),
                Path::new("/home"),
                &trigger
            )
            .is_err(),
            "accepted {time:?}"
        );
    }
    for time in ["00:00", "23:59"] {
        let trigger = WorkspaceCalendar::Daily { time: time.into() };
        let units = render(
            &job,
            &revision,
            Path::new("/grip"),
            Path::new("/home"),
            &trigger,
        )
        .unwrap();
        validate(
            &units,
            &job,
            &revision,
            Path::new("/grip"),
            Path::new("/home"),
            &trigger,
        )
        .unwrap();
    }
}
