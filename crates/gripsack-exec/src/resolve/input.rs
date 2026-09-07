//! Recipe and overlay identity from the same prepared source view (0041).

use crate::{ctx::ExecError, lockfile::Lockfile};
use gripsack_ir::{Ir, Module, StepAction, prepared::PreparedModule};
use gripsack_store as store;
use std::{collections::BTreeMap, path::Path};

fn without_spans(module: &Module) -> Module {
    let mut projected = module.clone();
    projected.span = None;
    for entry in projected.install.iter_mut().chain(&mut projected.config) {
        entry.span = None;
    }
    for dep in &mut projected.depends {
        dep.span = None;
    }
    for step in projected.steps.iter_mut().flatten() {
        step.span = None;
        if let StepAction::Install { entries } | StepAction::ConfigDeploy { entries } =
            &mut step.action
        {
            for entry in entries {
                entry.span = None;
            }
        }
    }
    projected
}

pub(crate) fn repo_overlay(
    plan: &PreparedModule,
    repo: &Path,
) -> Result<Option<String>, ExecError> {
    let froms: Vec<String> = plan
        .entries()
        .map(|e| &e.from)
        .filter(|from| repo.join(from).exists())
        .cloned()
        .collect();
    if froms.is_empty() {
        return Ok(None);
    }
    Ok(Some(
        store::canonical_overlay_hash(repo, &froms)?.to_string(),
    ))
}

pub(crate) fn module_input(
    name: &str,
    module: &Module,
    repo: &Path,
    ir: &Ir,
    lock: &Lockfile,
    plan: &PreparedModule,
) -> Result<String, ExecError> {
    let mut keys = BTreeMap::new();
    recipe(name, module, plan, repo, ir, lock, &mut keys)
}

fn recipe<'a>(
    name: &str,
    module: &Module,
    plan: &PreparedModule,
    repo: &Path,
    ir: &'a Ir,
    lock: &Lockfile,
    keys: &mut BTreeMap<&'a str, String>,
) -> Result<String, ExecError> {
    let mut input = serde_json::to_string(&without_spans(module))?;
    for entry in plan.entries() {
        let source = repo.join(&entry.from);
        if source.exists() {
            input.push_str(&format!(
                "|{}={}",
                entry.from,
                store::canonical_file_hash(&source)?
            ));
        }
    }
    // Scheduling prerequisites can also change a consumer's artifact. They
    // participate in keying, but never acquire deployment roles or PATH exports.
    for dep in gripsack_ir::dependencies::ordering_dependencies(name, module) {
        if let Some((key, dependency)) = ir.modules.get_key_value(dep) {
            if !keys.contains_key(key.as_str()) {
                let prepared = PreparedModule::new(dependency).map_err(ExecError::Gate)?;
                let child = recipe(key, dependency, &prepared, repo, ir, lock, keys)?;
                keys.insert(key.as_str(), store::hash::hex_sha256(child.as_bytes()));
            }
            input.push_str(&format!("|dep:{dep}={}", keys[key.as_str()]));
        }
        if let Some(pin) = lock.modules.get(dep).and_then(|e| e.resolved.as_ref()) {
            input.push_str(&format!(
                "|dep-pin:{dep}={}:{}:{}",
                pin.sha256.as_deref().unwrap_or("-"),
                pin.tree256.as_deref().unwrap_or("-"),
                pin.version.as_deref().unwrap_or("-")
            ));
        }
    }
    Ok(input)
}
