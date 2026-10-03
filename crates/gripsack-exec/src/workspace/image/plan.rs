use super::failure;
use crate::{
    ExecError,
    workspace::{
        artifact::{Artifact, Package},
        lowering::{Origin, SourceMap},
        realize::Realization,
        selection::Selection,
    },
};
use gripsack_buildkit::{
    identity::SnapshotDigest,
    plan::{
        Architecture, BuildPlan, ExporterPlan, ImageConfig, LinuxOs, Node, NodeIndex, Platform,
        ValidatedBuildPlan,
    },
};
use gripsack_ir::{
    workspace::{PlatformArch, PlatformOs},
    workspace_v6::{ImageDestination, ImageOutput, ImageOwner, WorkspaceArg, WorkspaceOutput},
};
use gripsack_process::Sha256Digest;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub(super) struct ImageIdentity(Sha256Digest);
impl std::fmt::Display for ImageIdentity {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(out)
    }
}
pub(super) struct Placement<'a, 'ir> {
    pub name: &'ir str,
    pub package: &'a Package,
    pub artifact: &'a Artifact,
    pub conda: Option<&'a crate::workspace::conda::CondaRuntimeReceipt>,
    pub destination: ImageDestination,
}
pub(super) struct ImagePlan<'a, 'ir> {
    pub plan: ValidatedBuildPlan,
    pub sources: BTreeMap<String, &'a Artifact>,
    pub origins: SourceMap<'ir>,
    pub placements: Vec<Placement<'a, 'ir>>,
    pub identity: ImageIdentity,
}

pub(super) fn placements<'a, 'ir>(
    image: &'ir ImageOutput,
    selection: &Selection<'ir>,
    realized: &'a Realization<'ir>,
) -> Result<Vec<Placement<'a, 'ir>>, ExecError> {
    let mut pending: Vec<&str> = image.packages.iter().map(String::as_str).collect();
    let mut selected = BTreeSet::new();
    let mut placements = Vec::new();
    while let Some(name) = pending.pop() {
        if !selected.insert(name) {
            continue;
        }
        let Some(WorkspaceOutput::Package(declaration)) = selection.outputs.get(name).copied()
        else {
            return Err(failure(image, "image selects a non-package output"));
        };
        pending.extend(declaration.runtime.iter().map(String::as_str));
        let package = realized
            .packages
            .get(name)
            .ok_or_else(|| failure(image, "image runtime package was not realized"))?;
        let destination =
            image
                .destinations
                .get(name)
                .cloned()
                .unwrap_or_else(|| ImageDestination {
                    path: gripsack_ir::workspace::InstallPrefix(format!("/opt/gripsack/{name}")),
                    owner: ImageOwner::default(),
                });
        placements.push(Placement {
            name,
            package,
            artifact: &package.producer,
            conda: None,
            destination,
        });
    }
    // Path order, not declaration/alias/import order, determines image layers.
    placements.sort_unstable_by(|left, right| {
        left.destination
            .path
            .as_str()
            .cmp(right.destination.path.as_str())
    });
    Ok(placements)
}

pub(super) fn compile<'a, 'ir>(
    image: &'ir ImageOutput,
    selection: &Selection<'ir>,
    realized: &'a Realization<'ir>,
    materialized: &'a super::materialize::Materializations,
) -> Result<ImagePlan<'a, 'ir>, ExecError> {
    if image.target.os != PlatformOs::Linux {
        return Err(failure(image, "OCI production requires a Linux target"));
    }
    let platform = Platform {
        os: LinuxOs::Linux,
        architecture: match image.target.arch {
            PlatformArch::X86_64 => Architecture::Amd64,
            PlatformArch::Aarch64 => Architecture::Arm64,
        },
    };
    let mut placements = placements(image, selection, realized)?;
    for placement in &mut placements {
        if placement.package.conda.is_some() {
            let source = materialized
                .get(placement.name)
                .ok_or_else(|| failure(image, "image Conda materialization is missing"))?;
            placement.artifact = &source.artifact;
            placement.conda = Some(&source.receipt);
        }
    }
    let mut nodes = Vec::new();
    let mut origins = SourceMap::default();
    let origin = Origin {
        output: &image.name,
        span: &image.span,
        line_map: &[],
    };
    let mut root = match &image.base {
        Some(reference) => {
            origins.push(origin);
            nodes.push(Node::Image {
                reference: reference.clone(),
            });
            Some(NodeIndex::new(0).map_err(|error| failure(image, error))?)
        }
        None => None,
    };
    let mut sources = BTreeMap::new();
    let mut locals = BTreeMap::new();
    for placement in &placements {
        let artifact = placement.artifact;
        let name = artifact.tree.to_string();
        let package_origin = Origin {
            output: placement.name,
            span: selection.outputs[placement.name].span(),
            line_map: &[],
        };
        let source = match locals.get(&name) {
            Some(&node) => {
                origins.note(node, package_origin);
                node
            }
            None => {
                let node = NodeIndex::new(nodes.len()).map_err(|error| failure(image, error))?;
                nodes.push(Node::Local {
                    name: name.clone(),
                    digest: SnapshotDigest::parse(&name).map_err(|error| failure(image, error))?,
                });
                origins.push(package_origin);
                origins.note(node, origin);
                sources.insert(name.clone(), artifact);
                locals.insert(name, node);
                node
            }
        };
        let node = NodeIndex::new(nodes.len()).map_err(|error| failure(image, error))?;
        nodes.push(Node::Install {
            input: root,
            source,
            source_path: "/".into(),
            destination: placement.destination.path.as_str().to_owned(),
            contents: true,
            uid: placement.destination.owner.uid,
            gid: placement.destination.owner.gid,
        });
        origins.push(origin);
        origins.note(node, package_origin);
        root = Some(node);
    }
    if root.is_none() {
        nodes.push(Node::Directory {
            input: None,
            path: "/".into(),
            mode: 0o755,
        });
        origins.push(origin);
        root = Some(NodeIndex::new(0).map_err(|error| failure(image, error))?);
    }
    let entrypoint = image
        .config
        .entrypoint
        .iter()
        .map(|argument| match argument {
            WorkspaceArg::Literal { value } => Ok(value.clone()),
            WorkspaceArg::PackageCommand {
                package,
                command,
                sha256,
            } => {
                let placement = placements
                    .iter()
                    .find(|entry| entry.name == package)
                    .ok_or_else(|| failure(image, "image command package is not selected"))?;
                let command = placement.package.commands.get(command).ok_or_else(|| {
                    failure(image, "image command is absent from the package receipt")
                })?;
                if let Some(claim) = sha256 {
                    let claim = gripsack_ir::workspace_v6::identity::ExecutableDigest::parse(claim)
                        .map_err(|error| failure(image, error))?;
                    // A supplied pin authenticates the selected native source command,
                    // not the different executable bytes relocated for this image.
                    if claim != command.executable {
                        return Err(failure(
                            image,
                            "image source-command pin differs from its validated native package",
                        ));
                    }
                }
                Ok(format!(
                    "{}/{}",
                    placement.destination.path.as_str(),
                    command.selector
                ))
            }
            _ => Err(failure(
                image,
                "host and production paths cannot enter OCI runtime configuration",
            )),
        })
        .collect::<Result<Vec<_>, ExecError>>()?;
    let mut env: Vec<String> = image
        .config
        .env
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect();
    if placements.iter().any(|placement| placement.conda.is_some()) {
        match image.config.env.get("PYTHONDONTWRITEBYTECODE") {
            Some(value) if value != "1" => {
                return Err(failure(
                    image,
                    "Conda image bytecode policy requires PYTHONDONTWRITEBYTECODE=1",
                ));
            }
            Some(_) => {}
            None => {
                env.push("PYTHONDONTWRITEBYTECODE=1".into());
                env.sort_unstable();
            }
        }
    }
    let config = ImageConfig {
        entrypoint,
        args: image.config.args.clone(),
        env,
        cwd: image.config.cwd.clone(),
        user: format!("{}:{}", image.config.user.uid, image.config.user.gid),
    };
    let plan = ValidatedBuildPlan::admit(BuildPlan {
        platform,
        nodes,
        root: root.expect("explicit root above"),
        exporter: ExporterPlan::Oci { config },
    })
    .map_err(|error| failure(image, error))?;
    let mut writer = IdentityWriter(Sha256::new());
    writer
        .0
        .update(b"gripsack-workspace-image-v2\0buildkit-v0.33.0\0");
    writer.0.update(plan.plan().exporter.digest().bytes());
    writer.0.update((placements.len() as u64).to_le_bytes());
    for placement in &placements {
        writer.0.update(placement.package.identity.bytes());
        serde_json::to_writer(&mut writer, &placement.artifact.tree)?;
        serde_json::to_writer(&mut writer, &placement.conda)?;
    }
    serde_json::to_writer(&mut writer, &image.target)?;
    serde_json::to_writer(&mut writer, &plan)?;
    let identity = ImageIdentity(Sha256Digest::from_bytes(writer.0.finalize().into()));
    Ok(ImagePlan {
        plan,
        sources,
        origins,
        placements,
        identity,
    })
}

struct IdentityWriter(Sha256);
impl Write for IdentityWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
