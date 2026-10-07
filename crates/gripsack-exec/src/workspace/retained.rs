//! Offline identity and receipt inspection. Absence is deferred; malformed or
//! mismatching retained evidence is an error. No acquisition/runtime authority.
use super::{
    artifact::{self, Artifact},
    definitions, inputs,
    pins::WorkspacePins,
    prepare::{self, Prepared, PreparedSource},
    realize::Realization,
    selection::Selection,
};
use crate::{ExecError, Repository};
use gripsack_ir::{
    FetchSpec, Ir,
    workspace::RecipeOutputKind,
    workspace_v6::{WorkspaceOutput, WorkspaceProducer, WorkspaceSourceV6, identity::CommandPins},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::Arc,
};

pub(super) fn inspect<'a>(
    ir: &'a Ir,
    repository: &Repository,
    home: &Path,
    origins: &BTreeSet<&str>,
    limits: gripsack_fetch::FetchLimits,
) -> Result<Realization<'a>, ExecError> {
    let mut realized = Realization {
        recipes: BTreeMap::new(),
        packages: BTreeMap::new(),
        inputs: BTreeMap::new(),
    };
    let Some(workspace) = &ir.workspace_v6 else {
        return Ok(realized);
    };
    if origins.is_empty() {
        return Ok(realized);
    }
    let definitions = definitions::captured_definitions(repository)?;
    let pins = WorkspacePins::read(repository)?;
    pins.admit_definitions(&definitions)?;
    for origin in origins {
        let selection = Selection::admit(workspace, &[(*origin).to_owned()])?;
        let mut sources = BTreeMap::new();
        let mut missing = false;
        for &name in &selection.required {
            let output = selection.outputs[name];
            let source = match output {
                WorkspaceOutput::Recipe(recipe) => Some((&recipe.source, &recipe.target)),
                WorkspaceOutput::Package(package) => match &package.producer {
                    WorkspaceProducer::Provider { provider } => Some((provider, &package.target)),
                    _ => None,
                },
                _ => None,
            };
            let Some((source, target)) = source else {
                continue;
            };
            let pin = pins.lookup(target, name, &source.locked())?;
            if let (WorkspaceSourceV6::PixiLock(declared), Some(pin)) = (source, pin) {
                let gripsack_ir::workspace_v6::LockedSource::PixiLock(locked) = &pin.source else {
                    return Err(super::file_failure(
                        output.span(),
                        "Pixi pin lost its locked source",
                    ));
                };
                for (input_name, expected) in [
                    (&declared.manifest, &locked.manifest_sha256),
                    (&declared.lock, &locked.lock_sha256),
                ] {
                    let input = workspace
                        .inputs
                        .iter()
                        .find(|input| input.name == *input_name)
                        .ok_or_else(|| {
                            super::file_failure(
                                output.span(),
                                "Pixi input is absent from the catalog",
                            )
                        })?;
                    let (_temporary, captured) =
                        inputs::capture_readonly(repository, home, input, limits)?;
                    if expected.as_deref() != Some(captured.identity.to_string().as_str()) {
                        return Err(super::file_failure(
                            &input.span,
                            format!(
                                "Pixi input {input_name:?} differs from its frozen identity; run grip update"
                            ),
                        ));
                    }
                }
            }
            let prepared = retained_source(repository, home, name, source, pin, limits)
                .map_err(|error| super::file_failure(output.span(), error))?;
            match prepared {
                Some(source) => {
                    sources.insert(name, source);
                }
                None => missing = true,
            }
        }
        // Still inspect every source above: a missing sibling cannot mask corrupt
        // frozen evidence. No identity is claimed with an unknown source.
        if missing {
            continue;
        }
        let mut needed = BTreeSet::new();
        for &name in &selection.required {
            prepare::collect_inputs(selection.outputs[name], &mut needed);
        }
        let mut captures = Vec::new();
        let mut captured = BTreeMap::new();
        let mut command_pins = CommandPins::default();
        for input in &workspace.inputs {
            if needed.contains(input.name.as_str()) {
                let (temporary, value) = inputs::capture_readonly(repository, home, input, limits)?;
                command_pins
                    .inputs
                    .insert(input.name.clone(), value.identity);
                captures.push(temporary);
                captured.insert(input.name.as_str(), value);
            }
        }
        let prepared = Prepared::identities(
            workspace,
            selection,
            &definitions,
            sources,
            captured,
            command_pins,
        )?;
        for (&name, identity) in &prepared.recipes {
            let WorkspaceOutput::Recipe(recipe) = prepared.selection.outputs[name] else {
                unreachable!();
            };
            if let Some(artifact) = artifact::retained_recipe(
                home,
                limits,
                *identity,
                recipe.output_kind,
                &recipe.execution,
                &prepared.retained_roots(home, name),
            )
            .map_err(|error| super::file_failure(&recipe.span, error))?
            {
                realized.recipes.insert(name, artifact);
            }
        }
        let mut pending: BTreeSet<_> = prepared.packages.keys().map(String::as_str).collect();
        while !pending.is_empty() {
            let before = pending.len();
            for name in pending.clone() {
                let WorkspaceOutput::Package(package) = prepared.selection.outputs[name] else {
                    unreachable!();
                };
                let producer = match &package.producer {
                    WorkspaceProducer::Recipe { recipe } => {
                        realized.recipes.get(recipe.as_str()).cloned()
                    }
                    WorkspaceProducer::Provider { .. } => {
                        Some(prepared.sources[name].artifact.clone())
                    }
                };
                let Some(producer) = producer else {
                    continue;
                };
                let runtime: Option<Vec<_>> = package
                    .runtime
                    .iter()
                    .map(|name| realized.packages.get(name.as_str()).cloned())
                    .collect();
                let Some(runtime) = runtime else {
                    continue;
                };
                let conda = prepared
                    .sources
                    .get(name)
                    .and_then(|source| source.conda.clone());
                if let Some(retained) = artifact::retained_package(
                    home,
                    prepared.packages[name],
                    package,
                    producer,
                    runtime,
                    conda,
                )
                .map_err(|error| super::file_failure(&package.span, error))?
                {
                    // Borrow names from the catalog, not the temporary identity map.
                    realized.packages.insert(package.name.as_str(), retained);
                }
                pending.remove(name);
            }
            if pending.len() == before {
                break;
            }
        }
    }
    Ok(realized)
}

fn retained_source(
    repository: &Repository,
    home: &Path,
    name: &str,
    source: &WorkspaceSourceV6,
    pin: Option<&gripsack_ir::workspace_v6::lock::LockedPin>,
    limits: gripsack_fetch::FetchLimits,
) -> Result<Option<PreparedSource>, ExecError> {
    let WorkspaceSourceV6::Fetch(fetch) = source else {
        let Some(pin) = pin else {
            return Ok(None);
        };
        let locked = pin
            .conda
            .as_ref()
            .ok_or_else(|| std::io::Error::other("Conda pin is missing its frozen closure"))?;
        super::conda::admit_frozen(name, source, locked)?;
        return super::conda::inspect_retained(name, home, locked).map(|materialized| {
            materialized.map(|value| PreparedSource {
                artifact: Arc::new(value.artifact),
                resolved: pin.resolved.clone(),
                conda: Some(value.receipt),
            })
        });
    };
    let mut resolved = pin.map(|pin| pin.resolved.clone());
    if let FetchSpec::File { path } = &fetch.fetch {
        let path = Path::new(path);
        let relative = repository.source_relative(path);
        if let Some(relative) = relative {
            let captured = repository.materialization_path(relative)?;
            if std::fs::symlink_metadata(&captured)?.is_dir() {
                let checked = super::stage::validate_output_tree(
                    &captured,
                    RecipeOutputKind::Tree,
                    name,
                    limits,
                )?;
                if resolved
                    .as_ref()
                    .and_then(|pin| pin.tree256.as_deref())
                    .is_some_and(|hash| hash != checked.tree_hash().as_str())
                {
                    return Err(std::io::Error::other(
                        "captured source differs from its frozen tree identity",
                    )
                    .into());
                }
                resolved.get_or_insert_default().tree256 = Some(checked.tree_hash().to_string());
            } else if let Some(expected) = resolved.as_ref().and_then(|pin| pin.sha256.as_ref()) {
                use sha2::{Digest, Sha256};
                use std::io::Read;
                let mut file = std::fs::File::open(&captured)?;
                let mut digest = Sha256::new();
                let mut bytes = [0; 64 * 1024];
                let mut size = 0u64;
                loop {
                    let count = file.read(&mut bytes)?;
                    if count == 0 {
                        break;
                    }
                    size += count as u64;
                    if size > limits.download_bytes.get() {
                        return Err(std::io::Error::other(
                            "captured source exceeds preview byte bound",
                        )
                        .into());
                    }
                    digest.update(&bytes[..count]);
                }
                if gripsack_process::Sha256Digest::from_bytes(digest.finalize().into())
                    != gripsack_process::Sha256Digest::parse(expected)?
                {
                    return Err(std::io::Error::other(
                        "captured source differs from its frozen download identity",
                    )
                    .into());
                }
            }
        }
    }
    let Some(resolved) = resolved else {
        return Ok(None);
    };
    let Some(tree) = &resolved.tree256 else {
        return Ok(None);
    };
    let root = gripsack_store::content_path(home, "workspace-source", tree);
    let Some(_directory) = artifact::retained_directory(home, &root)? else {
        return Ok(None);
    };
    let checked = super::stage::validate_output_tree(&root, RecipeOutputKind::Tree, name, limits)?;
    if checked.tree_hash().as_str() != tree {
        return Err(
            std::io::Error::other("retained source differs from its frozen tree identity").into(),
        );
    }
    Ok(Some(PreparedSource {
        artifact: Arc::new(Artifact {
            payload: root.clone(),
            retention: BTreeSet::from([root.clone()]),
            root,
            tree: checked.tree_hash().clone(),
        }),
        resolved,
        conda: None,
    }))
}
