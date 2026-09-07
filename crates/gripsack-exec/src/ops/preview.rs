//! The offline preview driver (0034): plan's codegen — the same
//! planners apply executes, run without fetching. Deferred payloads
//! render as marker ops; the decision pins at apply (0014).

use super::*;
use crate::ctx::ExecError;
use store::journal::Intended;

/// The plan-time driver (0034): ops for every declared destination
/// plus prune ops for undeclared ones — computed offline. Fetched
/// payloads can't hash without fetching: their ops render deferred
/// (the decision pins at apply, 0014's contract). `adopting` scopes
/// take-over to the adopt flow's destinations (0015 §7 S6).
pub fn preview_ops(
    ir: &gripsack_ir::Ir,
    repo: &Path,
    prev: Option<&store::Generation>,
    adopting: &std::collections::BTreeSet<String>,
    lock: &crate::lockfile::Lockfile,
) -> Result<Vec<Op>, ExecError> {
    let mut ops = Vec::new();
    // the destination-global lineage map (0030 §H4): the previous
    // generation's entry per physical destination
    let mut prev_map: std::collections::BTreeMap<PathBuf, &store::DeployedEntry> =
        std::collections::BTreeMap::new();
    if let Some(prev) = prev {
        for state in prev.modules.values() {
            for entry in &state.entries {
                if let Ok(key) = store::canonical_dest(&entry.to) {
                    prev_map.entry(key).or_insert(entry);
                }
            }
        }
    }
    let home = store::gripsack_home();
    let steps_by_module = crate::expand::expand_all(&ir.modules)?;
    let recipes = crate::resolve::RecipeGraph::new(
        ir,
        repo,
        &steps_by_module,
        ir.modules.keys().map(String::as_str),
    )?;
    // Build closures (0039): a build-only dep plans ZERO destination
    // ops — one marker line instead, naming the consumers. The same
    // whole-graph rule apply uses, so preview and apply agree.
    let build_only = gripsack_ir::dependencies::build_only_modules(&ir.modules);
    let mut build_consumers: std::collections::BTreeMap<&str, Vec<&str>> =
        std::collections::BTreeMap::new();
    for (name, module) in &ir.modules {
        for dep in &module.depends {
            if dep.edge == gripsack_ir::EdgeKind::Build {
                build_consumers
                    .entry(dep.module.as_str())
                    .or_default()
                    .push(name);
            }
        }
    }
    for (name, steps) in &steps_by_module {
        if build_only.contains(name) {
            // the closure line (0039): fetch + stage, never deploy
            let consumers = build_consumers
                .get(name.as_str())
                .map(|c| c.join(", "))
                .unwrap_or_default();
            ops.push(Op {
                module: name.clone(),
                dest: PathBuf::new(),
                declared_to: String::new(),
                mode: Ownership::Merge, // unused on marker ops
                kind: OpKind::RunEffect,
                authority: None,
                observed: None,
                intended: Intended::Removed,
                produces: None,
                note: Some(format!(
                    "{name}: fetch + stage for build ({consumers}) — not deployed"
                )),
                removing: None,
            });
            continue;
        }
        let locked = lock.modules.get(name);
        let identity = crate::identity::resolve(crate::identity::IdentityInputs {
            name,
            recipes: &recipes,
            plan: steps,
            home: &home,
            repo,
            locked,
            lock,
        })?;
        let version = locked
            .and_then(|entry| entry.resolved.as_ref())
            .and_then(|pin| pin.version.as_deref());
        let known = identity.present || (identity.content_addressed && steps.fetch().is_none());
        for step in steps.steps() {
            let entries: &[Entry] = match &step.action {
                gripsack_ir::StepAction::Install { entries }
                | gripsack_ir::StepAction::ConfigDeploy { entries } => entries,
                // opaque effects render as a marker op — never a
                // silent no-op (0033 R5)
                gripsack_ir::StepAction::Run { .. }
                | gripsack_ir::StepAction::CustomShell { .. } => {
                    ops.push(Op {
                        module: name.clone(),
                        dest: PathBuf::new(),
                        declared_to: String::new(),
                        mode: Ownership::Merge, // unused on marker ops
                        kind: OpKind::RunEffect,
                        authority: None,
                        observed: None,
                        intended: Intended::Removed,
                        produces: None,
                        note: Some(
                            "has run/shell steps — opaque effects, apply may change the system"
                                .to_string(),
                        ),
                        removing: None,
                    });
                    continue;
                }
                _ => continue,
            };
            for entry in entries {
                let dest = match store::canonical_dest(&entry.to) {
                    Ok(d) => d,
                    Err(e) => {
                        return Err(ExecError::Step {
                            module: name.clone(),
                            step: "plan".into(),
                            detail: format!("destination {:?}: {e}", entry.to),
                        });
                    }
                };
                // read-only observation (0035 F7): the preview must
                // never create a destination's parents
                let observed =
                    crate::deploy::observe_readonly(&dest).map_err(|e| ExecError::Step {
                        module: name.clone(),
                        step: "plan".into(),
                        detail: format!("cannot inspect {}: {e}", entry.to),
                    })?;
                let view = DestView {
                    module: name,
                    entry,
                    dest: dest.clone(),
                    home: &home,
                    observed,
                    prev: prev_map.get(&dest).copied(),
                    take_over: adopting.contains(entry.to.as_str()),
                };
                // the content question decides how much of the decision
                // plan can make offline
                let source =
                    crate::source::payload_source(&identity.store_path, &entry.from, version);
                if !known || source.relative.contains('{') {
                    ops.push(Op {
                        module: name.clone(),
                        dest,
                        declared_to: entry.to.clone(),
                        mode: entry.mode.clone(),
                        kind: OpKind::Deferred,
                        authority: None,
                        observed: view.observed_identity(),
                        intended: Intended::Removed,
                        produces: None,
                        note: Some("artifact → deploy (resolved at apply)".into()),
                        removing: None,
                    });
                    continue;
                }
                let repo_file = if identity.present {
                    source.path.clone()
                } else {
                    repo.join(&source.relative)
                };
                let deferred = match entry.mode {
                    Ownership::Merge => match std::fs::read_to_string(&repo_file) {
                        Ok(payload) => {
                            let existing = match std::fs::read_to_string(&dest) {
                                Ok(t) => Some(t),
                                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                                Err(e) => return Err(e.into()),
                            };
                            ops.push(plan_entry_op(
                                &view,
                                ModeInput::Merge {
                                    payload: &payload,
                                    existing,
                                    permissions: WritePermissions::Preserve,
                                },
                            )?);
                            continue;
                        }
                        Err(_) => true,
                    },
                    Ownership::TrackedCopy | Ownership::Template => match std::fs::read(&repo_file)
                    {
                        Ok(bytes) => {
                            let bytes = if entry.mode == Ownership::Template {
                                crate::template::render_template(&bytes, &entry.vars, &entry.from)?
                            } else {
                                bytes
                            };
                            #[cfg(unix)]
                            let exec = {
                                use std::os::unix::fs::PermissionsExt;
                                std::fs::metadata(&repo_file)
                                    .map(|m| m.permissions().mode() & 0o111 != 0)
                                    .unwrap_or(false)
                            };
                            #[cfg(not(unix))]
                            let exec = false;
                            return_op(
                                &mut ops,
                                &view,
                                ModeInput::Write {
                                    content: &bytes,
                                    permissions: WritePermissions::Source { executable: exec },
                                },
                            )?;
                            continue;
                        }
                        Err(_) => true,
                    },
                    Ownership::Owned => {
                        if repo_file.exists() {
                            let store_target = source.path.clone();
                            let already = std::fs::read_link(&dest)
                                .map(|t| t == store_target)
                                .unwrap_or(false);
                            return_op(
                                &mut ops,
                                &view,
                                ModeInput::Link {
                                    source: &store_target,
                                    content_hash: store::canonical_file_hash(&repo_file)
                                        .map_err(|e| ExecError::Step {
                                            module: name.clone(),
                                            step: "plan".into(),
                                            detail: format!("{e}"),
                                        })?
                                        .into(),
                                    already,
                                },
                            )?;
                            continue;
                        }
                        true
                    }
                };
                if !deferred {
                    continue;
                }
                let note = if steps.fetch().is_some() {
                    "fetch → deploy (pin-resolved at apply)"
                } else {
                    "steps → deploy"
                };
                ops.push(Op {
                    module: name.clone(),
                    dest,
                    declared_to: entry.to.clone(),
                    mode: entry.mode.clone(),
                    kind: OpKind::Deferred,
                    authority: None,
                    observed: view.observed_identity(),
                    intended: Intended::Removed,
                    produces: None,
                    note: Some(note.to_string()),
                    removing: None,
                });
            }
        }
    }
    // prunes: recorded destinations no longer declared (the remove
    // planner's gates — merge block intactness, drift — apply)
    if let Some(prev) = prev {
        // declared = the CANONICAL keys (0035 F1 + the deferred-prune
        // fix: a deferred op IS declared — its spelling just isn't
        // decided yet). Only marker ops (RunEffect) carry no dest.
        let declared: std::collections::BTreeSet<String> = ops
            .iter()
            .filter(|o| !matches!(o.kind, OpKind::RunEffect))
            .map(|o| o.dest.to_string_lossy().into_owned())
            .collect();
        for (name, state) in &prev.modules {
            for entry in &state.entries {
                if entry.preserved_drift
                    || declared.contains(&entry.key().to_string_lossy().into_owned())
                {
                    continue;
                }
                if let Some(op) = plan_remove_op(name, entry, &state.store_path, &home)? {
                    ops.push(op);
                }
            }
        }
    }
    Ok(ops)
}

fn return_op(ops: &mut Vec<Op>, view: &DestView, input: ModeInput) -> Result<(), ExecError> {
    ops.push(plan_entry_op(view, input)?);
    Ok(())
}
