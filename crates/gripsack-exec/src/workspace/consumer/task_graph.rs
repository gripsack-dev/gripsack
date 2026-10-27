//! Finite, invocation-local task graph admission, before realization or commands.
use super::{failure, gate};
use crate::{Ctx, ExecError};
use gripsack_ir::{
    Ir,
    workspace_model::{TaskOutput, WorkspaceMutationLock, WorkspaceOutput},
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
#[cfg(test)]
#[path = "task_graph_tests.rs"]
mod tests;

pub(super) const MAX_TASK_NODES: usize = 4096;
pub(super) const MAX_TASK_ACTIONS: usize = 16384;
const MAX_TASK_DEPTH: usize = 256;
const MAX_MUTATION_KEYS: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct TaskRevision(pub(super) gripsack_process::Sha256Digest);

#[derive(Serialize)]
pub struct TaskPlan {
    pub root: String,
    pub revision: TaskRevision,
    pub order: Vec<String>,
    pub mutation_locks: Vec<WorkspaceMutationLock>,
    pub host_effects: &'static str,
}

pub(super) struct TaskGraph<'a> {
    pub outputs: BTreeMap<&'a str, &'a WorkspaceOutput>,
    pub nodes: Vec<&'a TaskOutput>,
    pub plan: TaskPlan,
}

impl<'a> TaskGraph<'a> {
    pub fn admit(ir: &'a Ir, root: &str) -> Result<Self, ExecError> {
        let workspace = &ir.workspace;
        let outputs: BTreeMap<_, _> = workspace
            .outputs
            .iter()
            .map(|output| (output.name(), output))
            .collect();
        if outputs.len() != workspace.outputs.len() {
            return Err(gate(&workspace.span, "duplicate task graph output name"));
        }
        let root = outputs
            .get_key_value(root)
            .map(|(name, _)| *name)
            .ok_or_else(|| gate(&workspace.span, format!("no task named {root:?}")))?;
        let mut nodes = Vec::new();
        let mut done = BTreeSet::new();
        let mut path = Vec::new();
        visit(root, &outputs, &mut path, &mut done, &mut nodes)?;
        let mut locks = BTreeMap::new();
        let declared_locks: BTreeSet<_> = workspace
            .mutation_locks
            .iter()
            .map(|lock| (lock.scope, lock.key.as_str()))
            .collect();
        let mut actions = 0usize;
        for task in &nodes {
            if task.steps.is_empty() {
                return Err(gate(
                    &task.span,
                    "task requires at least one declared action",
                ));
            }
            actions = actions
                .checked_add(task.steps.len() + task.checks.len())
                .ok_or_else(|| failure("task action budget overflow"))?;
            if actions > MAX_TASK_ACTIONS {
                return Err(gate(&task.span, "task invocation exceeds action budget"));
            }
            if let Some(environment) = task.environment.as_deref()
                && !matches!(
                    outputs.get(environment),
                    Some(WorkspaceOutput::Environment(_))
                )
            {
                return Err(gate(
                    &task.span,
                    format!("missing task environment {environment:?}"),
                ));
            }
            for name in &task.checks {
                if !matches!(outputs.get(name.as_str()), Some(WorkspaceOutput::Check(_))) {
                    return Err(gate(
                        &task.span,
                        format!("missing task postcondition {name:?}"),
                    ));
                }
            }
            for lock in &task.mutation_locks {
                if !declared_locks.contains(&(lock.scope, lock.key.as_str())) {
                    return Err(gate(
                        &lock.span,
                        "task mutation lock has no workspace declaration",
                    ));
                }
                locks
                    .entry((lock.scope, lock.key.clone()))
                    .or_insert_with(|| lock.clone());
                if locks.len() > MAX_MUTATION_KEYS {
                    return Err(gate(
                        &lock.span,
                        "task invocation exceeds mutation-key budget",
                    ));
                }
            }
        }
        // Bind declarations, host context and selected environment/tool references,
        // never a run nonce or process observation. Source locations are diagnostic
        // metadata, not semantics. Existing wire has no runtime parameter override:
        // each named TaskOutput fully binds argv/context/environment.
        let selection =
            crate::workspace::selection::Selection::admit(workspace, &[root.to_owned()])?;
        let declarations: Vec<_> = selection
            .required
            .iter()
            .map(|name| outputs[name])
            .collect();
        let mut inputs: Vec<_> = workspace.inputs.iter().collect();
        inputs.sort_by(|left, right| left.name.cmp(&right.name));
        let mut binding = serde_json::to_value((&ir.host, declarations, inputs))?;
        strip_spans(&mut binding);
        let revision = TaskRevision(gripsack_process::Sha256Digest::of(&serde_json::to_vec(&(
            root, binding,
        ))?));
        let plan = TaskPlan {
            root: root.to_owned(),
            revision,
            order: nodes.iter().map(|node| node.name.clone()).collect(),
            mutation_locks: locks.into_values().collect(),
            host_effects: "unconfined host commands; declared mutable paths are review metadata, not a sandbox; effects are not rolled back",
        };
        Ok(Self {
            outputs,
            nodes,
            plan,
        })
    }
}

fn strip_spans(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(fields) => {
            fields.remove("span");
            if let Some(serde_json::Value::Array(deps)) = fields.get_mut("deps") {
                deps.sort_by(|left, right| left.as_str().cmp(&right.as_str()));
                deps.dedup();
            }
            if let Some(serde_json::Value::Array(locks)) = fields.get_mut("mutation_locks") {
                locks.sort_by(|left, right| {
                    (left["scope"].as_str(), left["key"].as_str())
                        .cmp(&(right["scope"].as_str(), right["key"].as_str()))
                });
            }
            for value in fields.values_mut() {
                strip_spans(value);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                strip_spans(value);
            }
        }
        _ => {}
    }
}

fn visit<'a>(
    name: &'a str,
    outputs: &BTreeMap<&'a str, &'a WorkspaceOutput>,
    path: &mut Vec<&'a str>,
    done: &mut BTreeSet<&'a str>,
    nodes: &mut Vec<&'a TaskOutput>,
) -> Result<(), ExecError> {
    if done.contains(name) {
        return Ok(());
    }
    let output = outputs
        .get(name)
        .ok_or_else(|| failure("task graph references an absent output"))?;
    let host_context = matches!(output, WorkspaceOutput::Task(task)
        if matches!(task.context, gripsack_ir::workspace_model::TaskContext::Host { .. }));
    if !gripsack_policy::task::admit_task_edge(
        matches!(output, WorkspaceOutput::Task(_)),
        host_context,
    ) {
        return Err(gate(
            output.span(),
            format!("task prerequisite {name:?} is not a task"),
        ));
    }
    let WorkspaceOutput::Task(task) = output else {
        unreachable!("edge admitted")
    };
    if let Some(start) = path.iter().position(|entry| *entry == name) {
        let mut cycle = path[start..].join(" -> ");
        cycle.push_str(" -> ");
        cycle.push_str(name);
        let mut diagnostic = gripsack_ir::Diagnostic::error(
            gripsack_ir::codes::WORKSPACE_EXEC_UNAVAILABLE,
            format!("task prerequisite cycle: {cycle}"),
        )
        .with_label(Some(task.span.clone()), "cycle closes here");
        if let Some(WorkspaceOutput::Task(previous)) =
            path.last().and_then(|name| outputs.get(name)).copied()
        {
            diagnostic =
                diagnostic.with_label(Some(previous.span.clone()), "prerequisite declared here");
        }
        return Err(ExecError::Gate(diagnostic));
    }
    if path.len() >= MAX_TASK_DEPTH {
        return Err(gate(
            &task.span,
            "task graph exceeds prerequisite depth budget",
        ));
    }
    if path.len() + done.len() >= MAX_TASK_NODES {
        return Err(gate(&task.span, "task graph exceeds node budget"));
    }
    path.push(name);
    let prerequisites: BTreeSet<_> = task.deps.iter().map(String::as_str).collect();
    for prerequisite in prerequisites {
        visit(prerequisite, outputs, path, done, nodes)?;
    }
    path.pop();
    done.insert(name);
    nodes.push(task);
    Ok(())
}

pub fn plan_task(ir: &Ir, task: &str) -> Result<TaskPlan, ExecError> {
    Ok(TaskGraph::admit(ir, task)?.plan)
}

pub(super) fn check_selection(ctx: &Ctx, task: &str) -> Result<(), ExecError> {
    if ctx.only.len() != 1 || ctx.only[0] != task {
        return Err(failure("task selection must name exactly its root output"));
    }
    Ok(())
}
