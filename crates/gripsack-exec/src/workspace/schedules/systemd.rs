//! Pure user-unit translation and independent admission of the actual unit bytes.
//! Publication/ownership belongs to the profile lifecycle, not this serializer.
mod manager;
pub use manager::{
    Capability, ObservedUnit, UnitProperties, UnitStatus, inspect, inspect_unit, status,
};
pub(super) use manager::{register, remove};
#[cfg(test)]
mod tests;

use crate::ExecError;
use gripsack_ir::workspace::{Weekday, WorkspaceCalendar};
use gripsack_store::jobs::{JobId, JobRevision};
use std::{collections::BTreeMap, path::Path};

/// Both strings are validated again by the lifecycle before protected publication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Units {
    pub service: String,
    pub timer: String,
}

// A pair contains only a dispatcher command and a single calendar, never task IR.
pub(super) const MAX_UNIT_BYTES: usize = 64 * 1024;

pub fn unit_names(job: &JobId) -> (String, String) {
    let base = format!("gripsack-job-{}", job.as_str());
    (format!("{base}.service"), format!("{base}.timer"))
}

pub fn render(
    job: &JobId,
    revision: &JobRevision,
    runner: &Path,
    home: &Path,
    trigger: &WorkspaceCalendar,
) -> Result<Units, ExecError> {
    let runner = executable_text(runner)?;
    let home = absolute_text(home)?;
    let (service_name, _) = unit_names(job);
    let mut command = String::new();
    // ':' is systemd's no-environment-expansion command prefix, not a shell.
    command.push_str("\":");
    quote_contents(runner, &mut command);
    command.push('"');
    for word in [
        "jobs",
        "--home",
        home,
        "dispatch",
        job.as_str(),
        revision.as_str(),
    ] {
        command.push(' ');
        quote_word(word, &mut command);
    }
    let time = match trigger {
        WorkspaceCalendar::Daily { time } | WorkspaceCalendar::Weekly { time, .. } => time,
    };
    admit_time(time)?;
    let weekday = match trigger {
        WorkspaceCalendar::Daily { .. } => "",
        WorkspaceCalendar::Weekly { weekday, .. } => match weekday {
            Weekday::Mon => "Mon ",
            Weekday::Tue => "Tue ",
            Weekday::Wed => "Wed ",
            Weekday::Thu => "Thu ",
            Weekday::Fri => "Fri ",
            Weekday::Sat => "Sat ",
            Weekday::Sun => "Sun ",
        },
    };
    let units = Units {
        service: format!(
            "[Unit]\nDescription=Gripsack scheduled task\n\n[Service]\nType=oneshot\nExecStart={command}\nRestart=no\nRemainAfterExit=no\nStandardInput=null\n"
        ),
        timer: format!(
            "[Unit]\nDescription=Gripsack local calendar\n\n[Timer]\nOnCalendar={weekday}*-*-* {time}:00\nPersistent=false\nUnit={service_name}\n\n[Install]\nWantedBy=timers.target\n"
        ),
    };
    if units.service.len() > MAX_UNIT_BYTES || units.timer.len() > MAX_UNIT_BYTES {
        return Err(failure("serialized schedule exceeds the unit byte budget"));
    }
    Ok(units)
}

fn absolute_text(path: &Path) -> Result<&str, ExecError> {
    let text = path
        .to_str()
        .ok_or_else(|| failure("schedule paths must be UTF-8"))?;
    if !path.is_absolute() || text.chars().any(char::is_control) {
        return Err(failure(
            "schedule paths must be absolute and contain no controls",
        ));
    }
    Ok(text)
}

fn executable_text(path: &Path) -> Result<&str, ExecError> {
    let text = absolute_text(path)?;
    // systemd 255 config_parse_exec() applies string_is_safe() after unquoting:
    // quotes/backslashes are forbidden in executables, but not argv values.
    // Require normal spelling because systemd simplifies executable paths.
    if text.contains(['\'', '"', '\\'])
        || text
            .split('/')
            .skip(1)
            .any(|part| matches!(part, "" | "." | ".."))
    {
        return Err(failure(
            "systemd runner path must be normalized and contain no quotes or backslashes",
        ));
    }
    Ok(text)
}

fn admit_time(time: &str) -> Result<(), ExecError> {
    let bytes = time.as_bytes();
    if bytes.len() != 5
        || bytes[2] != b':'
        || ![bytes[0], bytes[1], bytes[3], bytes[4]]
            .iter()
            .all(u8::is_ascii_digit)
        || &bytes[..2] > b"23".as_slice()
        || &bytes[3..] > b"59".as_slice()
    {
        return Err(failure("calendar time must be local HH:MM"));
    }
    Ok(())
}

fn quote_word(word: &str, target: &mut String) {
    target.push('"');
    quote_contents(word, target);
    target.push('"');
}

fn quote_contents(word: &str, target: &mut String) {
    for character in word.chars() {
        match character {
            '%' => target.push_str("%%"),
            '\\' => target.push_str("\\\\"),
            '"' => target.push_str("\\\""),
            other => target.push(other),
        }
    }
}

/// Decode the bytes, not the renderer's intermediates. This deliberately admits
/// only our narrow unit language: no drop-ins, continuations, repeated settings,
/// expansion, extra commands or extra execution/user-policy directives.
/// Lifecycle must bind these same bytes to its owned publication and reject
/// foreign manager fragments/drop-ins before registration.
pub fn validate(
    units: &Units,
    job: &JobId,
    revision: &JobRevision,
    runner: &Path,
    home: &Path,
    trigger: &WorkspaceCalendar,
) -> Result<(), ExecError> {
    let mut service = decode_unit(&units.service)?;
    take_exact(
        &mut service,
        "Unit",
        "Description",
        "Gripsack scheduled task",
    )?;
    take_exact(&mut service, "Service", "Type", "oneshot")?;
    take_exact(&mut service, "Service", "Restart", "no")?;
    take_exact(&mut service, "Service", "RemainAfterExit", "no")?;
    take_exact(&mut service, "Service", "StandardInput", "null")?;
    let command = service
        .remove(&("Service", "ExecStart"))
        .ok_or_else(|| failure("service has no dispatcher command"))?;
    let arguments = decode_command(command)?;
    let expected = [
        executable_text(runner)?,
        "jobs",
        "--home",
        absolute_text(home)?,
        "dispatch",
        job.as_str(),
        revision.as_str(),
    ];
    if arguments.iter().map(String::as_str).ne(expected) || !service.is_empty() {
        return Err(failure(
            "effective service command or policy differs from admitted schedule",
        ));
    }

    let mut timer = decode_unit(&units.timer)?;
    take_exact(&mut timer, "Unit", "Description", "Gripsack local calendar")?;
    take_exact(&mut timer, "Timer", "Persistent", "false")?;
    take_exact(&mut timer, "Timer", "Unit", &unit_names(job).0)?;
    take_exact(&mut timer, "Install", "WantedBy", "timers.target")?;
    let calendar = timer
        .remove(&("Timer", "OnCalendar"))
        .ok_or_else(|| failure("timer has no calendar"))?;
    if decode_calendar(calendar)? != *trigger || !timer.is_empty() {
        return Err(failure(
            "effective timer calendar or policy differs from admitted schedule",
        ));
    }
    Ok(())
}

type Directives<'a> = BTreeMap<(&'a str, &'a str), &'a str>;

fn decode_unit(bytes: &str) -> Result<Directives<'_>, ExecError> {
    if bytes.len() > MAX_UNIT_BYTES
        || !bytes.ends_with('\n')
        || bytes.chars().any(|c| c.is_control() && c != '\n')
    {
        return Err(failure(
            "unit bytes exceed budget or contain invalid controls/framing",
        ));
    }
    let mut section = "";
    let mut sections = std::collections::BTreeSet::new();
    let mut directives = BTreeMap::new();
    for line in bytes.lines().filter(|line| !line.is_empty()) {
        if let Some(header) = line
            .strip_prefix('[')
            .and_then(|line| line.strip_suffix(']'))
        {
            if !matches!(header, "Unit" | "Service" | "Timer" | "Install")
                || !sections.insert(header)
            {
                return Err(failure("unknown or repeated unit section"));
            }
            section = header;
        } else {
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| failure("invalid unit directive"))?;
            if section.is_empty() || directives.insert((section, key), value).is_some() {
                return Err(failure("unscoped or repeated unit directive"));
            }
        }
    }
    Ok(directives)
}

fn take_exact<'a>(
    map: &mut Directives<'a>,
    section: &'a str,
    key: &'a str,
    expected: &str,
) -> Result<(), ExecError> {
    if map.remove(&(section, key)) != Some(expected) {
        return Err(failure("unit directive differs from admitted policy"));
    }
    Ok(())
}

fn decode_command(command: &str) -> Result<Vec<String>, ExecError> {
    let mut input = command.chars().peekable();
    let mut words = Vec::new();
    while input.peek().is_some() {
        if input.next() != Some('"') {
            return Err(failure("dispatcher arguments must be explicitly quoted"));
        }
        let mut word = String::new();
        loop {
            match input.next() {
                Some('"') => break,
                Some('\\') => match input.next() {
                    Some(c @ ('\\' | '"')) => word.push(c),
                    _ => return Err(failure("unsupported command escape")),
                },
                Some('%') => {
                    if input.next() != Some('%') {
                        return Err(failure("systemd specifier expansion is forbidden"));
                    }
                    word.push('%');
                }
                Some(c) => word.push(c),
                None => return Err(failure("unterminated command argument")),
            }
        }
        if words.is_empty() {
            if !word.starts_with(":/") {
                return Err(failure(
                    "dispatcher must disable environment expansion and use an absolute executable",
                ));
            }
            word.remove(0);
        }
        words.push(word);
        match input.next() {
            None => break,
            Some(' ') if input.peek().is_some() => {}
            _ => return Err(failure("invalid dispatcher argument boundary")),
        }
        if words.len() >= 7 {
            return Err(failure("dispatcher has extra arguments"));
        }
    }
    Ok(words)
}

fn decode_calendar(calendar: &str) -> Result<WorkspaceCalendar, ExecError> {
    let mut fields = calendar.split(' ');
    let first = fields.next().unwrap_or("");
    let second = fields
        .next()
        .ok_or_else(|| failure("unsupported calendar grammar"))?;
    let (weekday, date, clock) = match fields.next() {
        None => (None, first, second),
        Some(clock) => {
            let day = match first {
                "Mon" => Weekday::Mon,
                "Tue" => Weekday::Tue,
                "Wed" => Weekday::Wed,
                "Thu" => Weekday::Thu,
                "Fri" => Weekday::Fri,
                "Sat" => Weekday::Sat,
                "Sun" => Weekday::Sun,
                _ => return Err(failure("unsupported calendar weekday")),
            };
            (Some(day), second, clock)
        }
    };
    if fields.next().is_some() {
        return Err(failure("unsupported calendar grammar"));
    }
    let time = clock
        .strip_suffix(":00")
        .ok_or_else(|| failure("calendar seconds must be zero"))?;
    admit_time(time)?;
    if date != "*-*-*" {
        return Err(failure("calendar must use every local date"));
    }
    Ok(match weekday {
        None => WorkspaceCalendar::Daily { time: time.into() },
        Some(weekday) => WorkspaceCalendar::Weekly {
            weekday,
            time: time.into(),
        },
    })
}

fn failure(detail: impl Into<String>) -> ExecError {
    ExecError::Step {
        output: "workspace".into(),
        step: "schedule-systemd".into(),
        detail: detail.into(),
    }
}
