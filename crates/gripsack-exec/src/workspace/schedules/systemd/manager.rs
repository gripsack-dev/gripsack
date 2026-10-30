//! Bounded systemctl transport. Inspection never reloads, enables, starts or
//! changes linger. Effects are private to the activation lifecycle adapter.
use super::{failure, unit_names};
use crate::ExecError;
use gripsack_process::{
    Control, Invocation, Limits, NativeInput, OperatorEnvironment, ProcessReceipt, ProcessRole,
    RetainedStderrLimit, SelectedProgram, StderrByteLimit, StdoutByteLimit,
};
use gripsack_store::jobs::{JobId, REGISTER_PROCESS_COUNT, REMOVE_PROCESS_COUNT};
use serde::Serialize;
use std::{
    ffi::OsStr,
    io,
    path::Path,
    time::{Duration, Instant},
};

// E0 qualified systemd 255; a newer release is not silently called qualified.
const QUALIFIED_VERSION: u32 = 255;
const MANAGER_TIMEOUT: Duration = Duration::from_secs(20);
const MANAGER_OUTPUT_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Capability {
    BinaryAbsent,
    UnqualifiedClient { version: u32 },
    UserManagerUnavailable,
    UnqualifiedManager { version: u32 },
    Available { version: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnitStatus {
    pub service: ObservedUnit,
    pub timer: ObservedUnit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnitProperties {
    pub load_state: String,
    pub active_state: String,
    pub sub_state: String,
    pub unit_file_state: String,
    pub result: Option<String>,
    pub fragment_path: String,
    /// Native systemctl formatting; nonempty values are never admitted as owned.
    pub drop_in_paths: String,
    /// Effective calendar specifications, excluding volatile next-elapse times.
    pub timers_calendar: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ObservedUnit {
    Observed {
        #[serde(flatten)]
        properties: UnitProperties,
    },
    Unknown {
        reason: String,
        properties: Option<UnitProperties>,
    },
}

/// Live read-only observations, never an inference from an installed manifest.
pub fn status(job: &JobId, directory: &Path) -> Result<UnitStatus, ExecError> {
    super::absolute_text(directory)?;
    let environment = OperatorEnvironment::capture()?;
    let deadline = Instant::now() + MANAGER_TIMEOUT;
    let selected = SelectedProgram::select(&environment, Path::new("systemctl"), None, deadline)?;
    let mut manager = Manager {
        environment: &environment,
        selected: &selected,
        deadline,
        receipts: Vec::new(),
    };
    let (service, timer) = unit_names(job);
    let mut observe = |name: &str| match manager.observe_unit(name, directory) {
        Ok(observation) => observation,
        Err(error) => ObservedUnit::Unknown {
            reason: error.to_string(),
            properties: None,
        },
    };
    Ok(UnitStatus {
        service: observe(&service),
        timer: observe(&timer),
    })
}

/// Read-only, closed-stdin inspection of both the client and the user manager.
/// A binary alone is never sufficient. Neither root nor linger is a fallback.
pub fn inspect() -> Result<Capability, ExecError> {
    let environment = OperatorEnvironment::capture()?;
    let deadline = Instant::now() + MANAGER_TIMEOUT;
    let selected =
        match SelectedProgram::select(&environment, Path::new("systemctl"), None, deadline) {
            Ok(selected) => selected,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(Capability::BinaryAbsent);
            }
            Err(error) => return Err(error.into()),
        };
    Manager {
        environment: &environment,
        selected: &selected,
        deadline,
        receipts: Vec::new(),
    }
    .inspect()
}

/// Refuse foreign fragments and every drop-in in the manager's effective view.
/// Registration repeats this after daemon-reload, before enabling the timer.
pub fn inspect_unit(job: &JobId, expected_unit_directory: &Path) -> Result<(), ExecError> {
    super::absolute_text(expected_unit_directory)?;
    let environment = OperatorEnvironment::capture()?;
    let deadline = Instant::now() + MANAGER_TIMEOUT;
    let selected = SelectedProgram::select(&environment, Path::new("systemctl"), None, deadline)?;
    Manager {
        environment: &environment,
        selected: &selected,
        deadline,
        receipts: Vec::new(),
    }
    .inspect_unit(job, expected_unit_directory)
}

struct Manager<'a> {
    environment: &'a OperatorEnvironment,
    selected: &'a SelectedProgram,
    deadline: Instant,
    receipts: Vec<ProcessReceipt>,
}

impl Manager<'_> {
    fn call<const N: usize>(
        &mut self,
        arguments: &[&str; N],
        role: ProcessRole,
    ) -> Result<(bool, Vec<u8>), ExecError> {
        let mut output = Vec::new();
        let arguments = arguments.map(OsStr::new);
        let outcome = Invocation::admit(
            self.environment,
            role,
            self.selected,
            Path::new("/"),
            Limits {
                timeout: MANAGER_TIMEOUT,
                operation_deadline: Some(self.deadline),
                stdout_bytes: StdoutByteLimit::new(MANAGER_OUTPUT_BYTES),
                stderr_bytes: StderrByteLimit::new(MANAGER_OUTPUT_BYTES),
                retained_stderr_bytes: RetainedStderrLimit::new(0),
                ..Limits::default()
            },
        )?
        .run(&arguments, NativeInput::Bytes(b""), None, |bytes| {
            output.extend_from_slice(bytes);
            Control::Continue
        })?;
        if outcome.receipt.disposition != gripsack_process::ProcessDisposition::Exited {
            return Err(failure(format!(
                "bounded user-manager operation ended with {:?}; registration state may be unknown",
                outcome.receipt.disposition
            )));
        }
        self.receipts.push(outcome.receipt);
        Ok((outcome.success, output))
    }

    fn inspect(&mut self) -> Result<Capability, ExecError> {
        let (success, output) = self.call(&["--version"], ProcessRole::Probe)?;
        if !success {
            return Err(failure("systemctl version inspection failed"));
        }
        let client = std::str::from_utf8(&output)
            .ok()
            .and_then(|text| text.strip_prefix("systemd "))
            .and_then(parse_version)
            .ok_or_else(|| failure("systemctl returned an unrecognized version"))?;
        if client != QUALIFIED_VERSION {
            return Ok(Capability::UnqualifiedClient { version: client });
        }
        let (success, output) = self.call(
            &[
                "--user",
                "--no-pager",
                "--no-ask-password",
                "show",
                "--property=Version",
                "--value",
            ],
            ProcessRole::Probe,
        )?;
        if !success {
            return Ok(Capability::UserManagerUnavailable);
        }
        let version = std::str::from_utf8(&output)
            .ok()
            .and_then(parse_version)
            .ok_or_else(|| failure("user manager returned an unrecognized version"))?;
        if version != QUALIFIED_VERSION {
            return Ok(Capability::UnqualifiedManager { version });
        }
        Ok(Capability::Available { version })
    }

    fn inspect_unit(&mut self, job: &JobId, directory: &Path) -> Result<(), ExecError> {
        let (service, timer) = unit_names(job);
        for name in [&service, &timer] {
            let (success, output) = self.call(
                &[
                    "--user",
                    "--no-pager",
                    "--no-ask-password",
                    "--all",
                    "show",
                    "--property=LoadState",
                    "--property=FragmentPath",
                    "--property=DropInPaths",
                    "--",
                    name,
                ],
                ProcessRole::Probe,
            )?;
            if !success {
                return Err(failure(
                    "cannot inspect effective user unit; no registration was authorized",
                ));
            }
            admit_unit_observation(&output, &directory.join(name))?;
        }
        Ok(())
    }

    fn observe_unit(&mut self, name: &str, directory: &Path) -> Result<ObservedUnit, ExecError> {
        let (success, output) = self.call(
            &[
                "--user",
                "--no-pager",
                "--no-ask-password",
                "--all",
                "show",
                "--property=LoadState",
                "--property=FragmentPath",
                "--property=DropInPaths",
                "--property=ActiveState",
                "--property=SubState",
                "--property=UnitFileState",
                "--property=Result",
                "--property=TimersCalendar",
                "--",
                name,
            ],
            ProcessRole::Probe,
        )?;
        if !success {
            return Err(failure("cannot observe user unit state"));
        }
        decode_status(&output, &directory.join(name))
    }

    fn effect<const N: usize>(&mut self, arguments: &[&str; N]) -> Result<(), ExecError> {
        if !self.call(arguments, ProcessRole::Hook)?.0 {
            return Err(failure(
                "systemctl user registration failed; reconcile the activation before retrying",
            ));
        }
        Ok(())
    }
}

fn parse_version(text: &str) -> Option<u32> {
    let end = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    text.get(..end)?.parse().ok()
}

fn decode_status(output: &[u8], expected: &Path) -> Result<ObservedUnit, ExecError> {
    let text =
        std::str::from_utf8(output).map_err(|_| failure("user unit inspection is not UTF-8"))?;
    let mut fields = std::collections::BTreeMap::new();
    let mut timers_calendar = Vec::new();
    for line in text.lines() {
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| failure("invalid user unit inspection"))?;
        if key == "TimersCalendar" {
            if value.is_empty() {
                continue;
            }
            // systemd 255 systemctl-show.c emits one record per calendar:
            // { OnCalendar=<spec> ; next_elapse=<timestamp> }.
            // Bind the effective spec, never the changing next trigger time.
            let calendar = value
                .strip_prefix("{ OnCalendar=")
                .and_then(|value| value.strip_suffix(" }"))
                .and_then(|value| value.split_once(" ; next_elapse="))
                .map(|(calendar, _)| calendar)
                .filter(|calendar| !calendar.is_empty())
                .ok_or_else(|| failure("unrecognized native calendar observation"))?;
            timers_calendar.push(calendar.to_owned());
        } else if fields.insert(key, value).is_some() {
            return Err(failure("repeated user unit inspection property"));
        }
    }
    let required = |name| {
        fields
            .get(name)
            .copied()
            .map(str::to_owned)
            .ok_or_else(|| failure("required user unit state missing"))
    };
    let properties = UnitProperties {
        load_state: required("LoadState")?,
        active_state: required("ActiveState")?,
        sub_state: required("SubState")?,
        unit_file_state: required("UnitFileState")?,
        fragment_path: required("FragmentPath")?,
        drop_in_paths: required("DropInPaths")?,
        result: fields
            .get("Result")
            .copied()
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
        timers_calendar,
    };
    for key in ["ActiveState", "SubState", "UnitFileState", "Result"] {
        fields.remove(key);
    }
    Ok(match admit_unit_properties(&mut fields, expected) {
        Ok(()) => ObservedUnit::Observed { properties },
        Err(error) => ObservedUnit::Unknown {
            reason: error.to_string(),
            properties: Some(properties),
        },
    })
}

fn parse_properties(output: &[u8]) -> Result<std::collections::BTreeMap<&str, &str>, ExecError> {
    let text =
        std::str::from_utf8(output).map_err(|_| failure("user unit inspection is not UTF-8"))?;
    let mut properties = std::collections::BTreeMap::new();
    for line in text.lines() {
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| failure("invalid user unit inspection"))?;
        if properties.insert(key, value).is_some() {
            return Err(failure("repeated user unit inspection property"));
        }
    }
    Ok(properties)
}

fn admit_unit_observation(output: &[u8], expected: &Path) -> Result<(), ExecError> {
    admit_unit_properties(&mut parse_properties(output)?, expected)
}

fn admit_unit_properties(
    properties: &mut std::collections::BTreeMap<&str, &str>,
    expected: &Path,
) -> Result<(), ExecError> {
    if properties.remove("DropInPaths") != Some("") {
        return Err(failure("schedule has foreign systemd drop-ins"));
    }
    let fragment = properties.remove("FragmentPath");
    let state = properties.remove("LoadState");
    let expected = super::absolute_text(expected)?;
    let owned_or_absent = match (state, fragment) {
        (Some("not-found"), Some("")) => true,
        (Some("loaded"), Some(path)) => path == expected,
        _ => false,
    };
    if !owned_or_absent || !properties.is_empty() {
        return Err(failure("schedule unit is foreign, masked or not loadable"));
    }
    Ok(())
}

/// Called only by schedules::lifecycle after existing activation admission and
/// owned-byte publication. This function never writes unit files or a journal.
pub(in crate::workspace::schedules) fn register(
    job: &JobId,
    directory: &Path,
) -> Result<Vec<ProcessReceipt>, ExecError> {
    change_registration(job, directory, true)
}

/// Disable only the timer: an already-admitted service may finish with its
/// retained revision. Lifecycle retires publication/roots under its own fence.
pub(in crate::workspace::schedules) fn remove(
    job: &JobId,
    directory: &Path,
) -> Result<Vec<ProcessReceipt>, ExecError> {
    change_registration(job, directory, false)
}

fn change_registration(
    job: &JobId,
    directory: &Path,
    enable: bool,
) -> Result<Vec<ProcessReceipt>, ExecError> {
    super::absolute_text(directory)?;
    let environment = OperatorEnvironment::capture()?;
    let deadline = Instant::now() + MANAGER_TIMEOUT;
    let selected = SelectedProgram::select(&environment, Path::new("systemctl"), None, deadline)?;
    let mut manager = Manager {
        environment: &environment,
        selected: &selected,
        deadline,
        receipts: Vec::with_capacity(if enable {
            REGISTER_PROCESS_COUNT
        } else {
            REMOVE_PROCESS_COUNT
        }),
    };
    if !matches!(manager.inspect()?, Capability::Available { .. }) {
        return Err(failure(
            "schedule registration requires the qualified systemd user manager; inspect scheduler capability",
        ));
    }
    let (_, timer) = unit_names(job);
    manager.inspect_unit(job, directory)?;
    if enable {
        manager.effect(&["--user", "--no-pager", "--no-ask-password", "daemon-reload"])?;
        manager.inspect_unit(job, directory)?;
    }
    manager.effect(&[
        "--user",
        "--no-pager",
        "--no-ask-password",
        if enable { "enable" } else { "disable" },
        "--now",
        "--",
        &timer,
    ])?;
    if !enable {
        manager.effect(&["--user", "--no-pager", "--no-ask-password", "daemon-reload"])?;
    }
    Ok(manager.receipts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observed_unit_requires_exact_fragment_and_no_dropins() {
        let path = Path::new("/home/operator/.config/systemd/user/example.timer");
        for output in [
            "LoadState=not-found\nFragmentPath=\nDropInPaths=\n",
            "LoadState=loaded\nFragmentPath=/home/operator/.config/systemd/user/example.timer\nDropInPaths=\n",
        ] {
            admit_unit_observation(output.as_bytes(), path).unwrap();
        }
        for output in [
            "LoadState=loaded\nFragmentPath=/usr/lib/systemd/user/example.timer\nDropInPaths=\n",
            "LoadState=loaded\nFragmentPath=/home/operator/.config/systemd/user/example.timer\nDropInPaths=/etc/systemd/user/service.d/override.conf\n",
            "LoadState=masked\nFragmentPath=/dev/null\nDropInPaths=\n",
            "LoadState=error\nFragmentPath=\nDropInPaths=\n",
            "LoadState=not-found\nFragmentPath=\n",
            "LoadState=not-found\nLoadState=loaded\nFragmentPath=\nDropInPaths=\n",
            "LoadState=not-found\nFragmentPath=\nDropInPaths=\nUnexpected=value\n",
        ] {
            assert!(
                admit_unit_observation(output.as_bytes(), path).is_err(),
                "{output:?}"
            );
        }
        assert!(admit_unit_observation(b"\xff", path).is_err());
    }

    #[test]
    fn status_binds_calendar_and_foreign_observations_without_countdowns() {
        let path = Path::new("/home/operator/.config/systemd/user/example.timer");
        let output = concat!(
            "LoadState=loaded\nActiveState=active\nSubState=waiting\nUnitFileState=enabled\n",
            "FragmentPath=/home/operator/.config/systemd/user/example.timer\nDropInPaths=\n",
            "Result=success\nTimersCalendar={ OnCalendar=Mon *-*-* 09:07:00 ; next_elapse=Mon 2026-11-02 09:07:00 UTC }\n",
        );
        let observation = decode_status(output.as_bytes(), path).unwrap();
        let later = output.replace("2026-11-02", "2026-11-09");
        assert_eq!(observation, decode_status(later.as_bytes(), path).unwrap());
        let changed = output.replace("OnCalendar=Mon", "OnCalendar=Tue");
        assert_ne!(
            observation,
            decode_status(changed.as_bytes(), path).unwrap()
        );
        let foreign = output.replace(
            "DropInPaths=\n",
            "DropInPaths=/etc/systemd/user/timer.d/override.conf\n",
        );
        let ObservedUnit::Unknown {
            properties: Some(properties),
            ..
        } = decode_status(foreign.as_bytes(), path).unwrap()
        else {
            panic!(
                "foreign manager configuration must retain observations without claiming ownership"
            )
        };
        assert_eq!(
            properties.drop_in_paths,
            "/etc/systemd/user/timer.d/override.conf"
        );
        assert_eq!(properties.timers_calendar, ["Mon *-*-* 09:07:00"]);
    }

    #[test]
    fn client_and_manager_version_formats_are_decoded() {
        assert_eq!(parse_version("255 (255.4-1ubuntu8)\n"), Some(255));
        assert_eq!(parse_version("255.4-1ubuntu8\n"), Some(255));
        assert_eq!(parse_version("256\n"), Some(256));
        assert_eq!(parse_version("unavailable"), None);
        assert_eq!(parse_version(""), None);
    }
}
