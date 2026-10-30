use super::*;

fn properties() -> systemd::UnitProperties {
    systemd::UnitProperties {
        load_state: "loaded".into(),
        active_state: "active".into(),
        sub_state: "waiting".into(),
        unit_file_state: "enabled".into(),
        result: Some("success".into()),
        fragment_path: "/owned/job.timer".into(),
        drop_in_paths: String::new(),
        timers_calendar: vec!["*-*-* 09:00:00".into()],
    }
}
fn status(properties: systemd::UnitProperties) -> systemd::UnitStatus {
    systemd::UnitStatus {
        service: systemd::ObservedUnit::Observed {
            properties: properties.clone(),
        },
        timer: systemd::ObservedUnit::Observed { properties },
    }
}
fn action(job: &JobId, directory: &Path, revision: char) -> ActivationAction {
    ActivationAction::ScheduledJob {
        job: job.clone(),
        revision: JobRevision::parse(&revision.to_string().repeat(64)).unwrap(),
        capture: "/retained/profile/job.json".into(),
        runner: "/retained/runner/grip".into(),
        unit_directory: directory.to_owned(),
        enabled: true,
        closure: vec!["/retained/runner".into()],
    }
}

#[test]
fn running_finished_and_failed_invocations_do_not_change_saved_authority() {
    let directory = tempfile::tempdir().unwrap();
    let job = JobId::parse(&"a".repeat(64)).unwrap();
    let installed = action(&job, directory.path(), 'b');
    let initial = status(properties());
    let expected = review_observation(&job, directory.path(), &initial, Some(&installed)).unwrap();
    for (active, sub, result) in [
        ("activating", "start", None),
        ("active", "running", None),
        ("inactive", "dead", Some("success")),
        ("failed", "failed", Some("exit-code")),
    ] {
        let observed = status(systemd::UnitProperties {
            active_state: active.into(),
            sub_state: sub.into(),
            result: result.map(str::to_owned),
            ..properties()
        });
        assert_ne!(
            serde_json::to_value(&initial).unwrap(),
            serde_json::to_value(&observed).unwrap(),
            "full display must retain changing runtime observations"
        );
        assert_eq!(
            expected,
            review_observation(&job, directory.path(), &observed, Some(&installed)).unwrap()
        );
    }
}

#[test]
fn registration_policy_and_capture_revision_remain_saved_authority() {
    let directory = tempfile::tempdir().unwrap();
    let job = JobId::parse(&"a".repeat(64)).unwrap();
    let installed = action(&job, directory.path(), 'b');
    let expected = review_observation(
        &job,
        directory.path(),
        &status(properties()),
        Some(&installed),
    )
    .unwrap();
    for changed in [
        systemd::UnitProperties {
            load_state: "not-found".into(),
            ..properties()
        },
        systemd::UnitProperties {
            unit_file_state: "disabled".into(),
            ..properties()
        },
        systemd::UnitProperties {
            fragment_path: "/other/job.timer".into(),
            ..properties()
        },
        systemd::UnitProperties {
            drop_in_paths: "/other/override.conf".into(),
            ..properties()
        },
        systemd::UnitProperties {
            timers_calendar: vec!["*-*-* 10:00:00".into()],
            ..properties()
        },
    ] {
        assert_ne!(
            expected,
            review_observation(&job, directory.path(), &status(changed), Some(&installed)).unwrap()
        );
    }
    let replacement = action(&job, directory.path(), 'c');
    assert_ne!(
        expected,
        review_observation(
            &job,
            directory.path(),
            &status(properties()),
            Some(&replacement)
        )
        .unwrap()
    );
    assert_ne!(
        expected,
        review_observation(&job, directory.path(), &status(properties()), None).unwrap()
    );
    let unknown = systemd::UnitStatus {
        service: systemd::ObservedUnit::Unknown {
            reason: "manager unavailable".into(),
            properties: None,
        },
        timer: systemd::ObservedUnit::Observed {
            properties: properties(),
        },
    };
    assert!(review_observation(&job, directory.path(), &unknown, Some(&installed)).is_err());
}

#[test]
fn unit_byte_drift_and_removal_stale_even_when_manager_properties_are_unchanged() {
    let directory = tempfile::tempdir().unwrap();
    let job = JobId::parse(&"a".repeat(64)).unwrap();
    let installed = action(&job, directory.path(), 'b');
    let (service, timer) = systemd::unit_names(&job);
    for name in [&service, &timer] {
        std::fs::write(directory.path().join(name), b"owned-unit").unwrap();
    }
    let observed = status(properties());
    let expected = review_observation(&job, directory.path(), &observed, Some(&installed)).unwrap();
    std::fs::write(directory.path().join(&timer), b"owned-unit\n# drift").unwrap();
    assert_ne!(
        expected,
        review_observation(&job, directory.path(), &observed, Some(&installed)).unwrap()
    );
    std::fs::remove_file(directory.path().join(&timer)).unwrap();
    assert_ne!(
        expected,
        review_observation(&job, directory.path(), &observed, Some(&installed)).unwrap()
    );
    std::os::unix::fs::symlink(&service, directory.path().join(&timer)).unwrap();
    assert!(review_observation(&job, directory.path(), &observed, Some(&installed)).is_err());
}
