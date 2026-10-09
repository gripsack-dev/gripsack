//! Managed projections re-admit mutable host runtime policy on every invocation.
use super::{ConsumerOutcome, admit, bind_admitted_program};
use crate::{ExecError, workspace::artifact};
use gripsack_ir::{Span, workspace_model::identity::PackageDigest};
use gripsack_process::{
    EnvironmentOverlay, Invocation, Limits, OperatorEnvironment, ProcessRole, Sha256Digest,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    io::Write,
    path::{Path, PathBuf},
    time::Instant,
};

const MAX_PROJECTION_BYTES: usize = 4 * 1024 * 1024;

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Projection {
    variables: Vec<String>,
    library_directories: Vec<PathBuf>,
}

/// Bind the names granted by an explicit declaration, not arbitrary ambient
/// variables. Values are inherited at invocation, preserving unset/empty and
/// shell-local overrides exactly as policy-free projections do.
pub(super) fn encode_projection(overlay: &EnvironmentOverlay) -> Result<String, ExecError> {
    let projection = Projection {
        variables: overlay
            .entries()
            .keys()
            .map(|name| {
                name.to_str()
                    .map(str::to_owned)
                    .ok_or_else(|| super::failure("projected variable name is not UTF-8"))
            })
            .collect::<Result<_, _>>()?,
        library_directories: overlay.library_prefix().to_vec(),
    };
    let mut encoded = serde_json::to_string(&projection)?;
    encoded.push('\n');
    if encoded.len() > MAX_PROJECTION_BYTES {
        return Err(super::failure(
            "projected environment exceeds its byte budget",
        ));
    }
    Ok(encoded)
}

fn read_projection(
    expected: Option<Sha256Digest>,
    deadline: Instant,
) -> Result<Option<Projection>, ExecError> {
    let Some(expected) = expected else {
        return Ok(None);
    };
    // The generated wrapper supplies a quoted here-document on fd3. This
    // carries no stdin or argument-size obligation and needs no sidecar path.
    let bytes = gripsack_process::InputBuffer::read_inherited(
        3,
        gripsack_process::InputByteLimit::new(MAX_PROJECTION_BYTES),
        deadline,
    )?
    .into_bytes();
    if Sha256Digest::of(&bytes) != expected {
        return Err(super::failure(
            "projected environment binding identity differs or exceeds its byte budget",
        ));
    }
    Ok(Some(serde_json::from_slice(&bytes)?))
}

fn projected_overlay(
    projection: Option<Projection>,
    command: &admit::AdmittedCommand,
    span: &Span,
) -> Result<EnvironmentOverlay, ExecError> {
    let mut entries = Vec::new();
    let mut libraries = command.library_dirs.clone();
    if let Some(projection) = projection {
        // Reuse the overlay's reserved-key/name validator even for unset keys.
        let names = EnvironmentOverlay::admit(
            projection
                .variables
                .into_iter()
                .map(|name| (OsString::from(name), OsString::new())),
            [],
            [],
        )?;
        for name in names.entries().keys() {
            if let Some(loader) = &command.gnu_loader {
                loader
                    .check_environment_key(name)
                    .map_err(|error| super::gate(span, error.to_string()))?;
            }
            if let Some(value) = std::env::var_os(name) {
                entries.push((name.clone(), value));
            }
        }
        libraries.extend(projection.library_directories);
    }
    admit::bytecode::apply(command.bytecode, &mut entries, span)?;
    EnvironmentOverlay::admit(entries, [], libraries).map_err(super::operational)
}

pub(super) struct Binding {
    core: PathBuf,
    home: PathBuf,
    package: PackageDigest,
    receipt: Sha256Digest,
}
impl Binding {
    pub(super) fn capture(
        home: &Path,
        package: &artifact::Package,
    ) -> Result<Option<Self>, ExecError> {
        if !package.host_dependent() {
            return Ok(None);
        }
        let mut receipts = BTreeMap::new();
        artifact::hook::capture(home, package, &mut receipts)?;
        Ok(Some(Self {
            core: std::env::current_exe()?,
            home: home.to_path_buf(),
            package: package.identity,
            receipt: receipts[&package.identity],
        }))
    }

    pub(super) fn write(
        &self,
        directory: &Path,
        name: &str,
        projection: &str,
    ) -> Result<(), ExecError> {
        use std::os::unix::fs::PermissionsExt;
        let core = self
            .core
            .to_str()
            .ok_or_else(|| super::failure("core path is not UTF-8"))?;
        let home = self
            .home
            .to_str()
            .ok_or_else(|| super::failure("home path is not UTF-8"))?;
        let mut wrapper = std::fs::File::create_new(directory.join(name))?;
        write!(
            wrapper,
            "#!/bin/sh\nexec {} __package-command --home {} --package {} --receipt {} --command {} --environment-sha256 {} -- \"$@\" 3<<'GRIP_PROJECTED_ENV'\n{}GRIP_PROJECTED_ENV\n",
            crate::env::ShellLiteral(core),
            crate::env::ShellLiteral(home),
            self.package,
            self.receipt,
            crate::env::ShellLiteral(name),
            Sha256Digest::of(projection.as_bytes()),
            projection
        )?;
        wrapper.set_permissions(std::fs::Permissions::from_mode(0o755))?;
        Ok(())
    }
}

/// Replay an exact package receipt, inheriting stdio and supervising exit/signal
/// semantics through the same native process boundary used by workspace tasks.
/// Neither a checkout nor a serialized loader plan contributes authority.
pub fn run_package_command(
    home: &Path,
    package: PackageDigest,
    receipt: Sha256Digest,
    command: &str,
    arguments: &[&OsStr],
    projection: Option<Sha256Digest>,
    environment: &OperatorEnvironment,
) -> Result<ConsumerOutcome, ExecError> {
    let deadline = Instant::now() + Limits::default().timeout;
    let projection = read_projection(projection, deadline)?;
    let session = crate::LifecycleSession::acquire(home)?;
    let package = artifact::hook::restore_launcher(home, package, receipt)?;
    let provided = package
        .commands
        .get(command)
        .ok_or_else(|| super::failure("retained package no longer exports launcher command"))?;
    let facts = crate::facts::detect();
    let facts = gripsack_ir::HostFacts {
        os: facts.os.into(),
        arch: facts.arch.into(),
        libc: facts.libc.clone(),
        tags: Vec::new(),
    };
    let host = admit::NativeContext::new(&facts, home, deadline)?;
    let span = Span {
        file: "<retained-package-command>".into(),
        line: 1,
        col: None,
    };
    let command = admit::admit_command(
        &package,
        &provided.selector,
        &provided.executable,
        &host,
        &span,
    )?;
    let options = crate::workspace::BuildOptions {
        environment,
        bridge: None,
        worker: Default::default(),
        deadline,
    };
    let selected = bind_admitted_program(&command, &options)?;
    let overlay = projected_overlay(projection, &command, &span)?;
    let mut closure = BTreeSet::new();
    package.retain_into(&mut closure);
    let retention = crate::workspace::roots::RetentionSet::admit(&session, closure)?;
    let lease = crate::workspace::roots::ProcessLease::register(&session, &retention, None)?;
    let handle = lease.duplicate_handle()?;
    drop(session);
    let outcome = Invocation::admit(
        environment,
        ProcessRole::Task,
        &selected,
        &std::env::current_dir()?,
        Limits {
            operation_deadline: Some(deadline),
            ..Limits::default()
        },
    )?
    .with_overlay(overlay)
    .retain_leases(gripsack_process::ProcessLeases {
        worker: None,
        retention: Some(handle),
    })?
    .run_interactive(arguments)?;
    let session = crate::LifecycleSession::acquire(home)?;
    let mut outcome = super::single_outcome(outcome);
    if let Err(error) = lease.release(&session, &outcome.receipt) {
        outcome.root_retained = true;
        tracing::warn!(%error, "package command root retained after unconfirmed cleanup");
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_absolute_core_never_falls_back_to_projected_path() {
        use std::os::unix::fs::PermissionsExt;
        let temporary = tempfile::tempdir().unwrap();
        let directory = temporary.path().canonicalize().unwrap();
        let shadow = directory.join("grip");
        // An exported command named grip must not acquire launcher authority.
        std::fs::write(&shadow, "#!/bin/sh\n: > \"$GRIP_SHADOW_MARKER\"\nexit 0\n").unwrap();
        std::fs::set_permissions(&shadow, std::fs::Permissions::from_mode(0o755)).unwrap();
        let binding = Binding {
            core: directory.join("removed core 'installation"),
            home: directory.join("state with spaces"),
            package: PackageDigest::parse(&"0".repeat(64)).unwrap(),
            receipt: Sha256Digest::of(b"receipt"),
        };
        binding
            .write(
                &directory,
                "tool",
                &encode_projection(&EnvironmentOverlay::default()).unwrap(),
            )
            .unwrap();
        let marker = directory.join("shadow-invoked");
        let result = std::process::Command::new(directory.join("tool"))
            .env_clear()
            .env("PATH", &directory)
            .env("GRIP_SHADOW_MARKER", &marker)
            .args(["", "two words", "--flag"])
            .output()
            .unwrap();
        assert!(
            !result.status.success(),
            "missing explicit core must fail closed"
        );
        assert!(
            !marker.exists(),
            "projected PATH must never rediscover the core"
        );
    }
}
