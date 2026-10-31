//! Bounded read-only bindings. Destination identity and permissions supplement
//! the planner's observation; mutation-time journal checks remain authoritative.
use super::{digest, invalid, require_equal};
use crate::{
    Ctx, ExecError,
    ops::{ContentSource, OpKind},
};
use gripsack_store as store;
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Read},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

const MAX_OBSERVATION_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct State {
    base: Option<String>,
    manifest: Option<String>,
    trust: Option<String>,
    sharing_trust: Option<String>,
    activation: Option<String>,
    operations: Vec<Operation>,
    packages: Vec<serde_json::Value>,
    schedules: serde_json::Value,
}

#[derive(Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Operation {
    output: String,
    declared: String,
    canonical: PathBuf,
    ownership: String,
    effect: String,
    content: Option<String>,
    mode: Option<u32>,
    destination: Observation,
    parents: Vec<(PathBuf, Identity)>,
}

#[derive(Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Identity {
    device: u64,
    inode: u64,
    mode: u32,
    uid: u32,
    gid: u32,
}
impl Identity {
    fn of(metadata: &std::fs::Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            mode: metadata.mode(),
            uid: metadata.uid(),
            gid: metadata.gid(),
        }
    }
}
#[derive(Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Observation {
    identity: Option<Identity>,
    bytes: Option<String>,
    link: Option<PathBuf>,
}

fn file_digest(path: &Path) -> Result<Option<String>, ExecError> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !file.metadata()?.is_file() {
        return Err(invalid("binding requires a regular file"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_OBSERVATION_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_OBSERVATION_BYTES {
        return Err(invalid("binding observation exceeds byte limit"));
    }
    Ok(Some(digest(&bytes)))
}
fn observe(path: &Path) -> Result<Observation, ExecError> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(Observation {
                identity: None,
                bytes: None,
                link: None,
            });
        }
        Err(error) => return Err(error.into()),
    };
    Ok(Observation {
        identity: Some(Identity::of(&metadata)),
        bytes: if metadata.is_file() {
            file_digest(path)?
        } else {
            None
        },
        link: if metadata.is_symlink() {
            Some(std::fs::read_link(path)?)
        } else {
            None
        },
    })
}
impl State {
    pub(super) fn capture(ir: &gripsack_ir::Ir, ctx: &Ctx) -> Result<Self, ExecError> {
        let current = store::current_generation(&ctx.home)?;
        let previous = current
            .map(|number| store::generations::admit_manifest(&ctx.home, number))
            .transpose()?;
        let selected: Vec<_> = if ctx.only.is_empty() {
            ir.workspace
                .outputs
                .iter()
                .filter_map(|output| match output {
                    gripsack_ir::workspace_model::WorkspaceOutput::Profile(profile) => {
                        Some(profile.name.clone())
                    }
                    _ => None,
                })
                .collect()
        } else {
            ctx.only.clone()
        };
        let selection = crate::workspace::selection::Selection::admit(&ir.workspace, &selected)?;
        let mut packages = Vec::new();
        for name in &selection.required {
            let output = selection.outputs[name];
            if let gripsack_ir::workspace_model::WorkspaceOutput::Package(package) = output
                && let gripsack_ir::workspace_model::WorkspaceProducer::Provider { provider } =
                    &package.producer
                && let gripsack_ir::workspace_model::AcquisitionSource::Package { manifest, .. } =
                    provider
            {
                let facts =
                    crate::workspace::sharing::inspect(&ctx.home, manifest)?.ok_or_else(|| {
                        invalid("saved package plan requires an admitted local import")
                    })?;
                packages.push(serde_json::to_value(facts)?);
            }
        }
        let ops = crate::ops::preview_ops(
            ir,
            &ctx.repository,
            previous.as_ref(),
            &Default::default(),
            ctx.fetch.limits(),
            &ctx.only,
        )?;
        let mut operations = Vec::with_capacity(ops.len());
        for op in ops {
            let (effect, content, mode) = match op.kind() {
                OpKind::Link { target } => {
                    ("link", Some(target.to_string_lossy().into_owned()), None)
                }
                OpKind::Write {
                    content: ContentSource::Bytes(bytes),
                    mode,
                } => ("write_exact", Some(digest(bytes)), Some(*mode)),
                OpKind::Write {
                    content: ContentSource::DeferredFetch,
                    mode,
                } => ("write_recipe_unknown", None, Some(*mode)),
                OpKind::MergeUpsert { payload, mode, .. } => {
                    ("merge_exact", Some(digest(payload)), Some(*mode))
                }
                OpKind::Remove(_) => ("remove", None, None),
                OpKind::Satisfied => ("satisfied", None, None),
                OpKind::Deferred => ("recipe_unknown", None, None),
                OpKind::Preserved => ("preserved", None, None),
            };
            let mut parents = Vec::new();
            for parent in op.dest().ancestors().skip(1) {
                match std::fs::metadata(parent) {
                    Ok(metadata) => parents.push((parent.to_path_buf(), Identity::of(&metadata))),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
            }
            operations.push(Operation {
                output: op.output().into(),
                declared: op.declared_to().into(),
                canonical: op.dest().to_path_buf(),
                ownership: format!("{:?}", op.mode()),
                effect: effect.into(),
                content,
                mode,
                destination: observe(op.dest())?,
                parents,
            });
        }
        Ok(Self {
            base: current.map(|number| number.to_string()),
            manifest: previous
                .as_ref()
                .map(|manifest| serde_json::to_vec(manifest).map(|bytes| digest(&bytes)))
                .transpose()?,
            trust: file_digest(&ctx.home.join("trust.toml"))?,
            sharing_trust: if packages.is_empty() {
                None
            } else {
                file_digest(&ctx.home.join("sharing/trust.json"))?
            },
            activation: file_digest(&ctx.home.join("activation.json"))?,
            operations,
            packages,
            schedules: crate::workspace::schedules::lifecycle::review_bindings(
                ir,
                &ctx.home,
                &ctx.only,
                previous.as_ref(),
            )?,
        })
    }
    pub(super) fn revalidate(&self, ir: &gripsack_ir::Ir, ctx: &Ctx) -> Result<(), ExecError> {
        let current = Self::capture(ir, ctx)?;
        require_equal(&self.base, &current.base, "base generation")?;
        require_equal(
            &self.manifest,
            &current.manifest,
            "ownership or installed job revision",
        )?;
        require_equal(&self.trust, &current.trust, "repository trust")?;
        require_equal(&self.sharing_trust, &current.sharing_trust, "package trust")?;
        require_equal(&self.activation, &current.activation, "activation state")?;
        require_equal(
            &self.packages,
            &current.packages,
            "package trust or imported artifact",
        )?;
        require_equal(
            &self.schedules,
            &current.schedules,
            "scheduler capability or registration",
        )?;
        require_equal(
            &self.operations,
            &current.operations,
            "destination identity, permissions, content, registration or operation scope",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn destination_replacement_mode_content_and_link_identity_are_bound() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("destination");
        std::fs::write(&path, b"reviewed").unwrap();
        let before = observe(&path).unwrap();
        assert!(before == observe(&path).unwrap());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let private = observe(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        assert!(private != observe(&path).unwrap());
        std::fs::write(&path, b"changed").unwrap();
        assert!(before != observe(&path).unwrap());
        let old = directory.path().join("old");
        std::fs::rename(&path, &old).unwrap();
        std::fs::write(&path, b"reviewed").unwrap();
        assert!(before != observe(&path).unwrap());
        std::fs::remove_file(&path).unwrap();
        symlink(&old, &path).unwrap();
        assert!(before != observe(&path).unwrap());
    }

    #[test]
    fn absent_destination_is_not_equal_to_new_foreign_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("destination");
        let absent = observe(&path).unwrap();
        std::fs::write(&path, b"foreign").unwrap();
        assert!(absent != observe(&path).unwrap());
    }
}
