//! Versioned, exact profile activation snapshots. Historical task projections
//! continue to grant ambient variable names; these snapshots never do.
use super::*;
use std::sync::Arc;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum Format {
    ProfileV1,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Activation {
    format: Format,
    pub(super) entries: BTreeMap<String, String>,
    pub(super) search_directories: Vec<PathBuf>,
    receipts: BTreeMap<PackageDigest, Sha256Digest>,
    commands: BTreeMap<String, PackageDigest>,
    retention: BTreeSet<PathBuf>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(super) arguments: BTreeMap<String, Vec<String>>,
}

pub(in crate::workspace::consumer) struct ProfileEnvironment<'a, C = admit::AdmittedCommand> {
    pub commands: &'a BTreeMap<String, C>,
    pub packages: &'a [Arc<artifact::Package>],
    pub entries: &'a [(OsString, OsString)],
    pub search_directories: &'a [PathBuf],
    pub library_directories: &'a [PathBuf],
    pub retention: &'a BTreeSet<PathBuf>,
}
impl<C> ProfileEnvironment<'_, C> {
    pub fn write_with_arguments(
        &self,
        home: &Path,
        directory: &Path,
        names: &[String],
        arguments: &BTreeMap<String, Vec<String>>,
    ) -> Result<String, ExecError> {
        validate_exports(names, self.commands)?;
        if arguments
            .keys()
            .any(|name| !self.commands.contains_key(name))
        {
            return Err(super::super::failure(
                "command arguments name an absent export",
            ));
        }
        let mut receipts = BTreeMap::new();
        let mut commands = BTreeMap::new();
        for package in self.packages {
            artifact::hook::capture(home, package, &mut receipts)?;
            for name in package.commands.keys() {
                if commands.insert(name.clone(), package.identity).is_some() {
                    return Err(super::super::failure(
                        "managed profile has duplicate command exports",
                    ));
                }
            }
        }
        let entries = self
            .entries
            .iter()
            .map(|(name, value)| {
                Ok((
                    name.to_str()
                        .ok_or_else(|| super::super::failure("profile variable name is not UTF-8"))?
                        .to_owned(),
                    value
                        .to_str()
                        .ok_or_else(|| {
                            super::super::failure("profile variable value is not UTF-8")
                        })?
                        .to_owned(),
                ))
            })
            .collect::<Result<_, ExecError>>()?;
        let projection = Projection {
            variables: Vec::new(),
            library_directories: self.library_directories.to_vec(),
            receipts: BTreeMap::new(),
            activation: Some(Activation {
                format: Format::ProfileV1,
                entries,
                search_directories: self.search_directories.to_vec(),
                receipts,
                commands,
                retention: self.retention.clone(),
                arguments: arguments.clone(),
            }),
        };
        let encoded = encode(&projection)?;
        let activation = projection
            .activation
            .as_ref()
            .expect("constructed activation");
        // Finish all receipt and binding admission before creating the first entry.
        let bindings = names
            .iter()
            .map(|name| {
                let identity = activation.commands.get(name).ok_or_else(|| {
                    super::super::failure("managed command is absent from retained exports")
                })?;
                Ok((
                    name,
                    Binding::retained(home, *identity, activation.receipts[identity])?,
                ))
            })
            .collect::<Result<Vec<_>, ExecError>>()?;
        for (name, binding) in bindings {
            binding.write(directory, name, &encoded)?;
        }
        Ok(encoded)
    }
}

fn validate_exports<T>(names: &[String], commands: &BTreeMap<String, T>) -> Result<(), ExecError> {
    let mut seen = BTreeSet::new();
    for name in names {
        if !seen.insert(name) {
            return Err(super::super::failure(&format!(
                "managed launcher command {name:?} is duplicated"
            )));
        }
        if !commands.contains_key(name) {
            return Err(super::super::failure(&format!(
                "managed launcher command {name:?} is not exported by profile.environment"
            )));
        }
    }
    Ok(())
}

impl Activation {
    /// Restore the captured native graph without granting executable admission.
    /// Its minimal roots are attached to an enclosing environment member so
    /// nested libraries/interpreters remain part of ordinary runtime traversal.
    pub(super) fn captured_packages(
        &self,
        home: &Path,
    ) -> Result<Vec<Arc<artifact::Package>>, ExecError> {
        let mut loaded = BTreeMap::new();
        let mut visiting = BTreeSet::new();
        for identity in self.receipts.keys() {
            artifact::hook::restore(home, *identity, &self.receipts, &mut loaded, &mut visiting)?;
        }
        let dependencies: BTreeSet<_> = loaded
            .values()
            .flat_map(|package| package.runtime.iter().map(|dependency| dependency.identity))
            .collect();
        Ok(loaded
            .into_iter()
            .filter_map(|(identity, package)| {
                (!dependencies.contains(&identity)).then_some(package)
            })
            .collect())
    }
    pub(super) fn imported_command(
        &self,
        home: &Path,
        name: &str,
        context: &admit::NativeContext<'_>,
        span: &Span,
        scope: &admit::RuntimeScope<'_>,
    ) -> Result<admit::AdmittedCommand, ExecError> {
        let identity = self
            .commands
            .get(name)
            .ok_or_else(|| super::super::failure("imported activation has no selected command"))?;
        let receipt = self
            .receipts
            .get(identity)
            .ok_or_else(|| super::super::failure("imported activation has no selected receipt"))?;
        let package = self.restore(home, *identity, *receipt)?;
        let provided = package.commands.get(name).ok_or_else(|| {
            super::super::failure("imported activation command differs from package")
        })?;
        let mut command = admit::admit_command_in_scope(
            &package,
            &provided.selector,
            &provided.executable,
            context,
            span,
            Some(scope),
            admit::EnvironmentResolution::DeferToEnclosing,
        )?;
        command.arguments.extend(
            self.arguments
                .get(name)
                .into_iter()
                .flatten()
                .map(OsString::from),
        );
        Ok(command)
    }
    pub(super) fn restore(
        &self,
        home: &Path,
        package: PackageDigest,
        receipt: Sha256Digest,
    ) -> Result<Arc<artifact::Package>, ExecError> {
        if self.receipts.get(&package) != Some(&receipt) {
            return Err(super::super::failure(
                "managed launcher receipt differs from its activation binding",
            ));
        }
        artifact::hook::restore(
            home,
            package,
            &self.receipts,
            &mut BTreeMap::new(),
            &mut BTreeSet::new(),
        )
    }

    pub(super) fn admit(
        &self,
        home: &Path,
        context: &admit::NativeContext<'_>,
        span: &Span,
        closure: &mut BTreeSet<PathBuf>,
    ) -> Result<(), ExecError> {
        let mut loaded = BTreeMap::new();
        let mut visiting = BTreeSet::new();
        let mut retained = BTreeSet::new();
        for (name, identity) in &self.commands {
            if !crate::workspace::profile_launchers::valid_command_name(name) {
                return Err(super::super::failure(
                    "managed activation contains an invalid command name",
                ));
            }
            let package = artifact::hook::restore(
                home,
                *identity,
                &self.receipts,
                &mut loaded,
                &mut visiting,
            )?;
            if retained.insert(*identity) {
                package.retain_into(closure);
            }
            let command = package.commands.get(name).ok_or_else(|| {
                super::super::failure("managed activation export differs from its retained package")
            })?;
            admit::admit_command_in_scope(
                &package,
                &command.selector,
                &command.executable,
                context,
                span,
                None,
                admit::EnvironmentResolution::DeferToEnclosing,
            )?;
        }
        closure.extend(self.retention.iter().cloned());
        Ok(())
    }

    pub(super) fn materialize(
        &self,
        home: &Path,
        lease: &crate::workspace::roots::ProcessLease,
        encoded: &str,
    ) -> Result<PathBuf, ExecError> {
        let home_dir = gripsack_fs::open(home)?;
        let parent_path = Path::new(crate::workspace::roots::BUILD_STAGING_DIRECTORY);
        gripsack_fs::create_dir_all(&home_dir, parent_path)?;
        let parent = gripsack_fs::open_dir_nofollow(&home_dir, parent_path)?;
        parent.create_dir(lease.id().as_str())?;
        let staging = gripsack_fs::open_dir_nofollow(&parent, Path::new(lease.id().as_str()))?;
        staging.create_dir("bin")?;
        gripsack_fs::fsync_pinned_dir(&parent, parent_path)?;
        let bin = home.join(parent_path).join(lease.id().as_str()).join("bin");
        self.write_commands(home, &bin, encoded)?;
        Ok(bin)
    }

    pub(super) fn write_commands(
        &self,
        home: &Path,
        bin: &Path,
        encoded: &str,
    ) -> Result<(), ExecError> {
        for (name, identity) in &self.commands {
            Binding::retained(home, *identity, self.receipts[identity])?
                .write(bin, name, encoded)?;
        }
        Ok(())
    }
}

pub(super) fn remove_staging(home: &Path, id: &str) -> Result<(), ExecError> {
    let home = gripsack_fs::open(home)?;
    let path = Path::new(crate::workspace::roots::BUILD_STAGING_DIRECTORY);
    let parent = match gripsack_fs::open_dir_nofollow(&home, path) {
        Ok(parent) => parent,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    match parent.remove_dir_all(id) {
        Ok(()) => gripsack_fs::fsync_pinned_dir(&parent, path)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_and_missing_exports_are_refused_before_writing() {
        let directory = tempfile::tempdir().unwrap();
        let commands: BTreeMap<String, admit::AdmittedCommand> = BTreeMap::new();
        let environment = ProfileEnvironment {
            commands: &commands,
            packages: &[],
            entries: &[],
            search_directories: &[],
            library_directories: &[],
            retention: &BTreeSet::new(),
        };
        assert!(
            environment
                .write_with_arguments(
                    directory.path(),
                    directory.path(),
                    &["missing".into()],
                    &BTreeMap::new()
                )
                .is_err()
        );
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
        let exports = BTreeMap::from([("tool".into(), ())]);
        assert!(validate_exports(&["tool".into(), "tool".into()], &exports).is_err());
    }

    #[test]
    fn exact_activation_requires_explicit_supported_format() {
        let activation = serde_json::json!({
            "entries": {}, "search_directories": [], "receipts": {}, "commands": {}, "retention": []
        });
        assert!(serde_json::from_value::<Activation>(activation.clone()).is_err());
        let mut unsupported = activation;
        unsupported["format"] = "profile_v2".into();
        assert!(serde_json::from_value::<Activation>(unsupported).is_err());
    }
}
