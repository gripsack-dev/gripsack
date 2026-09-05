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
    let steps_by_module = crate::expand::expand_all(&ir.modules);
    for (name, steps) in &steps_by_module {
        for step in steps {
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
                let (dest_dir, dest_name) = crate::deploy::dest_capability(&dest)?;
                let observed =
                    crate::deploy::observe(&dest_dir, &dest_name).map_err(|e| ExecError::Step {
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
                let repo_file = repo.join(&entry.from);
                let deferred = match entry.mode {
                    Ownership::Merge => match std::fs::read_to_string(&repo_file) {
                        Ok(payload) => {
                            let existing = match std::fs::read_to_string(&dest) {
                                Ok(t) => Some(t),
                                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                                Err(e) => return Err(e.into()),
                            };
                            #[cfg(unix)]
                            let dest_mode = {
                                use std::os::unix::fs::MetadataExt;
                                std::fs::metadata(&dest)
                                    .map(|m| m.mode() & 0o7777)
                                    .unwrap_or(0o644)
                            };
                            #[cfg(not(unix))]
                            let dest_mode = 0o644;
                            ops.push(plan_entry_op(
                                &view,
                                ModeInput::Merge {
                                    payload: &payload,
                                    existing,
                                    dest_mode,
                                },
                            )?);
                            continue;
                        }
                        Err(_) => true,
                    },
                    Ownership::Template => {
                        match std::fs::read(&repo_file).ok().and_then(|b| {
                            crate::template::render_template(&b, &entry.vars, &entry.from).ok()
                        }) {
                            Some(rendered) => {
                                let intent_mode = match &view.observed {
                                    Some(crate::deploy::Observation::File { mode, .. }) => *mode,
                                    _ => 0o644,
                                };
                                return_op(
                                    &mut ops,
                                    &view,
                                    ModeInput::Write {
                                        content: &rendered,
                                        intent_mode,
                                    },
                                )?;
                                continue;
                            }
                            None => true,
                        }
                    }
                    Ownership::TrackedCopy => match std::fs::read(&repo_file) {
                        Ok(bytes) => {
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
                                    intent_mode: if exec { 0o755 } else { 0o644 },
                                },
                            )?;
                            continue;
                        }
                        Err(_) => true,
                    },
                    Ownership::Owned => {
                        if repo_file.exists() {
                            let already = std::fs::read_link(&dest)
                                .map(|t| t == repo_file)
                                .unwrap_or(false);
                            return_op(
                                &mut ops,
                                &view,
                                ModeInput::Link {
                                    source: &repo_file,
                                    content_hash: store::canonical_file_hash(&repo_file)
                                        .map_err(|e| ExecError::Step {
                                            module: name.clone(),
                                            step: "plan".into(),
                                            detail: format!("{e}"),
                                        })?
                                        .to_string(),
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
                let note = if ir.modules[name].fetch.is_some() {
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
        let declared: std::collections::BTreeSet<String> = ops
            .iter()
            .filter(|o| !matches!(o.kind, OpKind::RunEffect | OpKind::Deferred))
            .map(|o| o.declared_to.clone())
            .collect();
        for (name, state) in &prev.modules {
            for entry in &state.entries {
                if entry.preserved_drift || declared.contains(entry.to.as_str()) {
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
