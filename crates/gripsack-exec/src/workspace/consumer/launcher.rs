//! Retained launchers re-admit native packages on every invocation.
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

mod profile;
pub(super) use profile::ProfileEnvironment;

/// Complete multi-package activation metadata, including fixed arguments and
/// retained receipt graphs. Encoder, inherited-frame reader and stored
/// projection readers share this bound; it is not one manifest's size limit.
pub(in crate::workspace) const MAX_PROJECTION_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Projection {
    variables: Vec<String>,
    library_directories: Vec<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    activation: Option<profile::Activation>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    receipts: BTreeMap<PackageDigest, Sha256Digest>,
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
        activation: None,
        receipts: BTreeMap::new(),
    };
    encode(&projection)
}

fn encode(projection: &Projection) -> Result<String, ExecError> {
    let mut encoded = gripsack_process::InputBuffer::new(gripsack_process::InputByteLimit::new(
        MAX_PROJECTION_BYTES,
    ));
    serde_json::to_writer(&mut encoded, projection).map_err(|error| {
        if error.io_error_kind() == Some(std::io::ErrorKind::InvalidInput) {
            super::failure("projected environment exceeds its 32 MiB byte budget")
        } else {
            ExecError::from(error)
        }
    })?;
    encoded.write_all(b"\n")?;
    String::from_utf8(encoded.into_bytes())
        .map_err(|error| super::failure(&format!("projected environment is not UTF-8: {error}")))
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

pub(in crate::workspace) fn admit_imported_command(
    home: &Path,
    encoded: &str,
    name: &str,
    context: &admit::NativeContext<'_>,
    span: &Span,
    scope: &admit::RuntimeScope<'_>,
    environment: admit::EnvironmentResolution,
) -> Result<admit::AdmittedCommand, ExecError> {
    let projection: Projection = serde_json::from_str(encoded)?;
    let mut command = projection
        .activation
        .as_ref()
        .ok_or_else(|| super::failure("imported environment requires exact activation"))?
        .imported_command(home, name, context, span, scope)?;
    if environment == admit::EnvironmentResolution::ApplySelected {
        // This activation already contains exactly the ordered selected
        // contributions. Unselected child-only keys must not survive recursion.
        let activation = projection
            .activation
            .as_ref()
            .ok_or_else(|| super::failure("imported environment lost exact activation"))?;
        command.entries = activation
            .entries
            .iter()
            .map(|(key, value)| (OsString::from(key), OsString::from(value)))
            .collect();
        command.imported = Some(std::sync::Arc::new(ImportedEnvironment {
            home: home.into(),
            projection,
        }));
    }
    Ok(command)
}

pub(in crate::workspace) fn captured_imported_packages(
    admission: &mut crate::workspace::sharing::ReadmissionScope,
    encoded: &str,
) -> Result<Vec<std::sync::Arc<artifact::Package>>, ExecError> {
    let projection: Projection = serde_json::from_str(encoded)?;
    projection
        .activation
        .ok_or_else(|| super::failure("imported environment requires exact activation"))?
        .captured_packages(admission)
}

/// A capture is not native authority. Complete top-level acquisition and every
/// invocation re-enter the common native path after composition is available.
pub(in crate::workspace) fn admit_imported_environment(
    home: &Path,
    encoded: &str,
    context: &admit::NativeContext<'_>,
    span: &Span,
) -> Result<(), ExecError> {
    let projection: Projection = serde_json::from_str(encoded)?;
    projection
        .activation
        .ok_or_else(|| super::failure("imported environment requires exact activation"))?
        .admit(home, context, span, &mut BTreeSet::new())
}

pub(in crate::workspace) fn imported_entries(
    encoded: &str,
) -> Result<Vec<(OsString, OsString)>, ExecError> {
    let projection: Projection = serde_json::from_str(encoded)?;
    Ok(projection
        .activation
        .ok_or_else(|| super::failure("imported environment requires exact activation"))?
        .entries
        .into_iter()
        .map(|(key, value)| (key.into(), value.into()))
        .collect())
}

/// The existing exact activation protocol, carried with an admitted imported
/// command so recipient overlays can be composed before nested PATH consumption.
pub(in crate::workspace) struct ImportedEnvironment {
    home: PathBuf,
    projection: Projection,
}

impl ImportedEnvironment {
    fn composed(&self, overlay: &EnvironmentOverlay) -> Result<Projection, ExecError> {
        let mut projection = self.projection.clone();
        let activation = projection
            .activation
            .as_mut()
            .ok_or_else(|| super::failure("imported environment lost exact activation"))?;
        for (key, value) in overlay.entries() {
            activation.entries.insert(
                key.to_str()
                    .ok_or_else(|| super::failure("environment key is not UTF-8"))?
                    .into(),
                value
                    .to_str()
                    .ok_or_else(|| super::failure("environment value is not UTF-8"))?
                    .into(),
            );
        }
        activation.search_directories = overlay.search_prefix().to_vec();
        projection.library_directories = overlay.library_prefix().to_vec();
        Ok(projection)
    }

    fn materialize(
        &self,
        lease: &crate::workspace::roots::ProcessLease,
        overlay: EnvironmentOverlay,
    ) -> Result<EnvironmentOverlay, ExecError> {
        let projection = self.composed(&overlay)?;
        let encoded = encode(&projection)?;
        let bin = projection
            .activation
            .as_ref()
            .expect("exact activation")
            .materialize(&self.home, lease, &encoded)?;
        with_imported_search(overlay, bin)
    }
}

fn with_imported_search(
    overlay: EnvironmentOverlay,
    bin: PathBuf,
) -> Result<EnvironmentOverlay, ExecError> {
    let mut search = overlay.search_prefix().to_vec();
    search.insert(usize::from(!search.is_empty()), bin);
    EnvironmentOverlay::admit(
        overlay
            .entries()
            .iter()
            .map(|(key, value)| (key.clone(), value.clone())),
        search,
        overlay.library_prefix().iter().cloned(),
    )
    .map_err(super::operational)
}

pub(in crate::workspace) fn compose_imported(
    command: Option<&admit::AdmittedCommand>,
    overlay: EnvironmentOverlay,
) -> Result<EnvironmentOverlay, ExecError> {
    let Some(imported) = command.and_then(|command| command.imported.as_ref()) else {
        return Ok(overlay);
    };
    // Read-only hook capture has no process staging and must not create one.
    let Some(parent) = overlay.search_prefix().first() else {
        return Ok(overlay);
    };
    let directory = tempfile::Builder::new()
        .prefix("imported-")
        .tempdir_in(parent)?
        .keep();
    let projection = imported.composed(&overlay)?;
    let encoded = encode(&projection)?;
    projection
        .activation
        .as_ref()
        .expect("exact activation")
        .write_commands(&imported.home, &directory, &encoded)?;
    with_imported_search(overlay, directory)
}

fn projected_overlay(
    projection: Option<Projection>,
    command: &admit::AdmittedCommand,
    span: &Span,
) -> Result<EnvironmentOverlay, ExecError> {
    let mut entries = command.entries.clone();
    let mut libraries = command.library_dirs.clone();
    let mut search = Vec::new();
    if let Some(projection) = projection {
        if let Some(activation) = projection.activation {
            entries.extend(
                activation
                    .entries
                    .into_iter()
                    .map(|(name, value)| (OsString::from(name), OsString::from(value))),
            );
            search = activation.search_directories;
            if let Some(loader) = &command.gnu_loader {
                for (name, _) in &entries {
                    loader
                        .check_environment_key(name)
                        .map_err(|error| super::gate(span, error.to_string()))?;
                }
            }
        } else {
            // Historical projections grant names, never captured ambient values.
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
        }
        libraries.extend(projection.library_directories);
    }
    admit::bytecode::apply(command.bytecode, &mut entries, span)?;
    EnvironmentOverlay::admit(entries, search, libraries).map_err(super::operational)
}

pub(super) struct Binding {
    core: PathBuf,
    home: PathBuf,
    package: PackageDigest,
    receipt: Sha256Digest,
    receipts: BTreeMap<PackageDigest, Sha256Digest>,
}
impl Binding {
    pub(super) fn capture(
        home: &Path,
        package: &artifact::Package,
    ) -> Result<Option<Self>, ExecError> {
        if !package.host_dependent() && !package.portable() {
            return Ok(None);
        }
        let mut receipts = BTreeMap::new();
        artifact::hook::capture(home, package, &mut receipts)?;
        let mut binding = Self::retained(home, package.identity, receipts[&package.identity])?;
        binding.receipts = receipts;
        Ok(Some(binding))
    }

    fn retained(
        home: &Path,
        package: PackageDigest,
        receipt: Sha256Digest,
    ) -> Result<Self, ExecError> {
        Ok(Self {
            core: std::env::current_exe()?.canonicalize()?,
            home: home.to_path_buf(),
            package,
            receipt,
            receipts: BTreeMap::new(),
        })
    }

    pub(super) fn write(
        &self,
        directory: &Path,
        name: &str,
        projection: &str,
    ) -> Result<(), ExecError> {
        use std::os::unix::fs::PermissionsExt;
        let encoded;
        let projection = if self.receipts.is_empty() {
            projection
        } else {
            let mut value: Projection = serde_json::from_str(projection)?;
            value.receipts = self.receipts.clone();
            encoded = encode(&value)?;
            &encoded
        };
        let core = self
            .core
            .to_str()
            .ok_or_else(|| super::failure("core path is not UTF-8"))?;
        let home = self
            .home
            .to_str()
            .ok_or_else(|| super::failure("home path is not UTF-8"))?;
        let repair = format!(
            "gripsack: bound core {core} is missing or not executable; reinstall grip and reapply the declaring profile to regenerate its managed launchers"
        );
        let mut wrapper = std::fs::File::create_new(directory.join(name))?;
        write!(
            wrapper,
            "#!/bin/sh\nif [ ! -x {} ]; then printf '%s\\n' {} >&2; exit 127; fi\nexec {} __package-command --home {} --package {} --receipt {} --command {} --environment-sha256 {} -- \"$@\" 3<<'GRIP_PROJECTED_ENV'\n{}GRIP_PROJECTED_ENV\n",
            crate::env::ShellLiteral(core),
            crate::env::ShellLiteral(&repair),
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
    let mut projection = read_projection(projection, deadline)?;
    if projection.as_ref().is_some_and(|projection| {
        projection.activation.is_some() && !projection.variables.is_empty()
    }) {
        return Err(super::failure(
            "exact profile activation cannot grant ambient variable names",
        ));
    }
    let session = crate::LifecycleSession::acquire(home)?;
    let package = match projection
        .as_ref()
        .and_then(|projection| projection.activation.as_ref())
    {
        Some(activation) => {
            let mut admission = crate::workspace::sharing::ReadmissionScope::new(
                home,
                gripsack_fetch::FetchLimits::default(),
            );
            activation.restore(&mut admission, package, receipt)?
        }
        None => match projection
            .as_ref()
            .filter(|value| !value.receipts.is_empty())
        {
            Some(value) => {
                if value.receipts.get(&package) != Some(&receipt) {
                    return Err(super::failure(
                        "launcher root differs from its captured receipt graph",
                    ));
                }
                let mut admission = crate::workspace::sharing::ReadmissionScope::new(
                    home,
                    gripsack_fetch::FetchLimits::default(),
                );
                artifact::hook::ReceiptRestoration::new(&value.receipts, &mut admission)
                    .restore(package)?
            }
            None => artifact::hook::restore_launcher(home, package, receipt)?,
        },
    };
    let default_arguments = projection
        .as_ref()
        .and_then(|projection| projection.activation.as_ref())
        .and_then(|activation| activation.arguments.get(command))
        .cloned()
        .unwrap_or_default();
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
        deadline,
    };
    let selected = bind_admitted_program(&command, &options)?;
    let mut closure = BTreeSet::new();
    package.retain_into(&mut closure);
    if let Some(activation) = projection
        .as_ref()
        .and_then(|projection| projection.activation.as_ref())
    {
        activation.admit(home, &host, &span, &mut closure)?;
    }
    host.readmission(home)?
        .readmit_roots(closure.iter().map(PathBuf::as_path))?;
    let retention = crate::workspace::roots::RetentionSet::admit(&session, closure)?;
    let lease = crate::workspace::roots::ProcessLease::register(&session, &retention)?;
    let staged = if let Some(projected) = projection
        .as_ref()
        .filter(|projected| projected.activation.is_some())
    {
        let encoded = encode(projected)?;
        let activation = projected.activation.as_ref().expect("filtered activation");
        Some(activation.materialize(home, &lease, &encoded)?)
    } else {
        None
    };
    if let Some(bin) = staged {
        projection
            .as_mut()
            .and_then(|projected| projected.activation.as_mut())
            .expect("staged activation")
            .search_directories
            .insert(0, bin);
    }
    let overlay = projected_overlay(projection, &command, &span)?;
    let overlay = if let Some(imported) = &command.imported {
        if overlay.search_prefix().is_empty() {
            imported.materialize(&lease, overlay)?
        } else {
            compose_imported(Some(&command), overlay)?
        }
    } else {
        overlay
    };
    let handle = lease.duplicate_handle()?;
    drop(session);
    let arguments: Vec<&OsStr> = command
        .arguments
        .iter()
        .map(OsString::as_os_str)
        .chain(default_arguments.iter().map(OsStr::new))
        .chain(arguments.iter().copied())
        .collect();
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
    .run_interactive(&arguments)?;
    let session = crate::LifecycleSession::acquire(home)?;
    let mut outcome = super::single_outcome(outcome);
    let staging_id = lease.id().as_str().to_owned();
    if let Err(error) = lease.release(&session, &outcome.receipt) {
        outcome.root_retained = true;
        tracing::warn!(%error, "package command root retained after unconfirmed cleanup");
    }
    if !outcome.root_retained {
        profile::remove_staging(home, &staging_id).unwrap_or_else(|error| {
            tracing::warn!(%error, "completed launcher staging could not be removed");
        });
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imported_activation_composes_recipient_values_without_ambient_grants() {
        let source = serde_json::json!({
            "variables": [], "library_directories": [],
            "activation": {
                "format": "profile_v1",
                "entries": {"SHARED": "publisher", "IMPORTED_ONLY": "$(not shell)"},
                "search_directories": [], "receipts": {}, "commands": {}, "retention": [],
                "arguments": {"tool": ["", "two words", "--fixed"]}
            }
        });
        let imported = ImportedEnvironment {
            home: PathBuf::from("/recipient"),
            projection: serde_json::from_value(source).unwrap(),
        };
        let overlay = EnvironmentOverlay::admit(
            [("SHARED", "recipient"), ("TASK_ONLY", "literal $value")]
                .map(|(key, value)| (OsString::from(key), OsString::from(value))),
            [PathBuf::from("/recipient/leased/bin")],
            [],
        )
        .unwrap();
        let encoded = encode(&imported.composed(&overlay).unwrap()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(value["variables"], serde_json::json!([]));
        assert_eq!(
            value["activation"]["entries"],
            serde_json::json!({
                "SHARED": "recipient", "IMPORTED_ONLY": "$(not shell)", "TASK_ONLY": "literal $value"
            })
        );
        assert_eq!(
            value["activation"]["arguments"]["tool"],
            serde_json::json!(["", "two words", "--fixed"])
        );
        assert_eq!(
            value["activation"]["search_directories"],
            serde_json::json!(["/recipient/leased/bin"])
        );
    }

    #[test]
    fn complete_activation_metadata_is_not_limited_to_one_manifest() {
        let arguments: BTreeMap<_, _> = (0..128)
            .map(|index| (format!("tool-{index}"), vec!["x".repeat(4096); 12]))
            .collect();
        let projection: Projection = serde_json::from_value(serde_json::json!({
            "variables": [], "library_directories": [],
            "activation": {
                "format": "profile_v1", "entries": {}, "search_directories": [],
                "receipts": {}, "commands": {}, "retention": [], "arguments": arguments,
            }
        }))
        .unwrap();
        let encoded = encode(&projection).unwrap();
        assert!(encoded.len() > 4 * 1024 * 1024);
        assert!(encoded.len() <= MAX_PROJECTION_BYTES);
        let restored: Projection = serde_json::from_str(&encoded).unwrap();
        assert_eq!(
            serde_json::to_value(restored).unwrap(),
            serde_json::to_value(projection).unwrap()
        );
    }

    #[test]
    fn activation_encoding_enforces_its_bound_before_returning_transport_bytes() {
        let projection = Projection {
            variables: vec!["x".repeat(MAX_PROJECTION_BYTES)],
            library_directories: Vec::new(),
            activation: None,
            receipts: BTreeMap::new(),
        };
        assert!(
            encode(&projection)
                .unwrap_err()
                .to_string()
                .contains("byte budget")
        );
    }

    #[test]
    fn historical_exact_activation_retains_its_wire_shape() {
        let source = serde_json::json!({
            "variables": [], "library_directories": [],
            "activation": {
                "format": "profile_v1", "entries": {}, "search_directories": [],
                "receipts": {}, "commands": {}, "retention": []
            }
        });
        let projection: Projection = serde_json::from_value(source.clone()).unwrap();
        assert_eq!(serde_json::to_value(&projection).unwrap(), source);
    }

    #[test]
    fn historical_projection_grants_names_without_capturing_values() {
        let overlay = EnvironmentOverlay::admit(
            [(OsString::from("EXPLICIT"), OsString::from("not frozen"))],
            [],
            [],
        )
        .unwrap();
        let encoded = encode_projection(&overlay).unwrap();
        let projection: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(projection["variables"], serde_json::json!(["EXPLICIT"]));
        assert!(projection.get("activation").is_none());
        assert!(!encoded.contains("not frozen"));
    }

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
            receipts: BTreeMap::new(),
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
        assert!(
            String::from_utf8(result.stderr)
                .unwrap()
                .contains("reapply")
        );
    }
}
