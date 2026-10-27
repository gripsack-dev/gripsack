//! Explicit profile command exports use the ordinary tracked-copy transaction:
//! immutable content, drift refusal, generation retention and rollback all have
//! the same owner as captured profile files.
use super::{consumer::admit::EnvironmentPlan, content::PreparedFile, file_failure};
use crate::ExecError;
use gripsack_ir::workspace_model::ProfileOutput;
use gripsack_store::{DeploymentEntry, Ownership};
use std::{collections::BTreeSet, path::Path};

/// Logical targets participate in the same global collision/preview pass as
/// declared files. This is also checked by preparation for direct IR callers.
pub(super) fn destinations(profile: &ProfileOutput) -> Result<Vec<String>, ExecError> {
    let Some(launchers) = &profile.launchers else {
        return Ok(Vec::new());
    };
    let directory = &launchers.directory;
    let relative = directory
        .strip_prefix("~/")
        .or_else(|| directory.strip_prefix('/'));
    if relative.is_none_or(|relative| {
        relative
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    }) || directory.contains('\\')
        || directory.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(file_failure(
            &profile.span,
            "launcher directory must be a normalized absolute or ~/ path",
        ));
    }
    if profile.environment.is_none() || launchers.commands.is_empty() {
        return Err(file_failure(
            &profile.span,
            "managed launchers require an environment and nonempty commands",
        ));
    }
    let mut seen = BTreeSet::new();
    launchers
        .commands
        .iter()
        .map(|name| {
            if !valid_command_name(name) || !seen.insert(name) {
                return Err(file_failure(
                    &profile.span,
                    "launcher commands must be unique plain names",
                ));
            }
            Ok(format!("{directory}/{name}"))
        })
        .collect()
}

pub(super) fn valid_command_name(name: &str) -> bool {
    !name.is_empty()
        && !matches!(name, "." | "..")
        && !name.contains(['/', '\\'])
        && !name.bytes().any(|byte| byte <= b' ' || byte == 127)
}

pub(super) fn prepare(
    profile: &ProfileOutput,
    environment: &EnvironmentPlan,
    home: &Path,
    stage: &Path,
    files: &mut Vec<PreparedFile>,
) -> Result<(), ExecError> {
    let targets = destinations(profile)?;
    let Some(launchers) = &profile.launchers else {
        return Ok(());
    };
    // Resolve every export before even creating a temporary command entry.
    for command in &launchers.commands {
        if !environment.commands.contains_key(command) {
            return Err(file_failure(
                &profile.span,
                format!(
                    "managed launcher command {command:?} is not exported by profile.environment"
                ),
            ));
        }
    }
    let directory = stage.join("managed-launchers");
    std::fs::create_dir(&directory)?;
    environment.write_managed_commands(home, &directory, &launchers.commands)?;
    files.extend(
        launchers
            .commands
            .iter()
            .zip(targets)
            .map(|(command, target)| PreparedFile {
                entry: DeploymentEntry {
                    from: format!("managed-launchers/{command}"),
                    to: target,
                    mode: Ownership::TrackedCopy,
                    vars: Default::default(),
                    marker: None,
                    span: Some(profile.span.clone()),
                },
                block_id: None,
            }),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(directory: &str, commands: &[&str]) -> ProfileOutput {
        serde_json::from_value(serde_json::json!({
            "name":"home", "span":{"file":"workspace.ts", "line":1},
            "environment":"tools", "launchers":{"directory":directory,"commands":commands}
        }))
        .unwrap()
    }

    #[test]
    fn hostile_directory_quotes_are_literal_targets() {
        let declaration = profile("/tmp/owner's tools;$(touch nope)", &["tool"]);
        assert_eq!(
            destinations(&declaration).unwrap(),
            ["/tmp/owner's tools;$(touch nope)/tool"]
        );
    }

    #[test]
    fn malformed_and_duplicate_targets_are_rejected() {
        for directory in [
            "relative",
            "~",
            "/",
            "~/",
            "~/a/../bin",
            "/tmp//bin",
            "/tmp/bin/",
        ] {
            assert!(
                destinations(&profile(directory, &["tool"])).is_err(),
                "{directory}"
            );
        }
        for commands in [
            vec!["tool", "tool"],
            vec!["../tool"],
            vec![""],
            vec!["a b"],
            vec![],
        ] {
            assert!(destinations(&profile("~/.local/bin", &commands)).is_err());
        }
    }

    #[test]
    fn missing_export_leaves_staging_untouched() {
        let stage = tempfile::tempdir().unwrap();
        let mut files = Vec::new();
        let result = prepare(
            &profile("~/.local/bin", &["missing"]),
            &EnvironmentPlan::empty(),
            stage.path(),
            stage.path(),
            &mut files,
        );
        assert!(result.is_err());
        assert!(files.is_empty());
        assert_eq!(std::fs::read_dir(stage.path()).unwrap().count(), 0);
    }
}
