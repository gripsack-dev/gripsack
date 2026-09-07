//! One lowering boundary for both module authoring styles (0041).
//! Consumers borrow projections; they never rediscover declarative fields.

use crate::{Build, Diagnostic, Entry, FetchSpec, Module, Phase, Step, StepAction, Verify, codes};
use std::collections::BTreeSet;

#[derive(Debug, Clone)]
pub struct PreparedModule {
    steps: Vec<Step>,
    module_verify: Option<Verify>,
}

impl PreparedModule {
    pub fn new(module: &Module) -> Result<Self, Diagnostic> {
        let steps = module
            .steps
            .clone()
            .unwrap_or_else(|| declarative_steps(module));
        let mut remaining: Vec<Option<Step>> = steps.into_iter().map(Some).collect();
        let mut ordered = Vec::with_capacity(remaining.len());
        let ids: BTreeSet<String> = remaining.iter().flatten().map(|s| s.id.clone()).collect();
        let mut done = BTreeSet::new();
        while ordered.len() < remaining.len() {
            let mut progressed = false;
            for slot in &mut remaining {
                let ready = slot.as_ref().is_some_and(|s| {
                    s.needs.iter().all(|need| {
                        need.contains(':') || !ids.contains(need) || done.contains(need)
                    })
                });
                if ready {
                    let step = slot.take().expect("ready step");
                    done.insert(step.id.clone());
                    ordered.push(step);
                    progressed = true;
                }
            }
            if !progressed {
                return Err(
                    Diagnostic::error(codes::STEP_CYCLE, "cycle in step dependencies")
                        .with_label(module.span.clone(), "module declared here"),
                );
            }
        }
        Ok(Self {
            steps: ordered,
            module_verify: module.verify.clone(),
        })
    }

    pub fn target_phase(&self, id: &str) -> Option<Phase> {
        if id == crate::step::BARRIER_STEP_ID {
            return Some(Phase::Verify);
        }
        self.steps
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.action.execution_phase())
            .or_else(|| (id == "verify" && self.module_verify.is_some()).then_some(Phase::Verify))
    }

    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    pub fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.steps.iter().flat_map(|s| match &s.action {
            StepAction::Install { entries } | StepAction::ConfigDeploy { entries } => {
                entries.as_slice()
            }
            _ => &[],
        })
    }

    pub fn config_entries(&self) -> impl Iterator<Item = &Entry> {
        self.steps.iter().flat_map(|s| match &s.action {
            StepAction::ConfigDeploy { entries } => entries.as_slice(),
            _ => &[],
        })
    }

    pub fn fetch(&self) -> Option<&FetchSpec> {
        self.steps.iter().find_map(|s| match &s.action {
            StepAction::Fetch { fetch } => Some(fetch),
            _ => None,
        })
    }

    /// All pre-flip contracts, each declaration once. Module checks are not
    /// synthetic steps: they cannot collide with an author's chosen step id.
    pub fn checks(&self) -> impl Iterator<Item = &Verify> {
        self.steps
            .iter()
            .flat_map(|step| {
                let action = match &step.action {
                    StepAction::Verify { verify } => Some(verify),
                    _ => None,
                };
                action.into_iter().chain(step.verify.iter())
            })
            .chain(self.module_verify.iter())
    }

    pub fn has_recipe(&self) -> bool {
        self.steps.iter().any(|s| {
            matches!(
                s.action,
                StepAction::Build { .. } | StepAction::Run { .. } | StepAction::CustomShell { .. }
            )
        })
    }
}

fn declarative_steps(module: &Module) -> Vec<Step> {
    let mut steps: Vec<Step> = Vec::new();
    let mut push = |id: &str, action: StepAction, phase: Phase| {
        let needs = steps.last().map(|s| vec![s.id.clone()]).unwrap_or_default();
        steps.push(Step {
            id: id.into(),
            action,
            needs,
            resources: vec![],
            phase: Some(phase),
            verify: None,
            span: module.span.clone(),
        });
    };
    if let Some(fetch) = &module.fetch {
        push(
            "fetch",
            StepAction::Fetch {
                fetch: fetch.clone(),
            },
            Phase::Fetch,
        );
    }
    if module.build != Build::None {
        push(
            "build",
            StepAction::Build {
                spec: module.build.clone(),
            },
            Phase::Build,
        );
    }
    if !module.install.is_empty() {
        push(
            "install",
            StepAction::Install {
                entries: module.install.clone(),
            },
            Phase::Install,
        );
    }
    if !module.config.is_empty() {
        push(
            "config",
            StepAction::ConfigDeploy {
                entries: module.config.clone(),
            },
            Phase::Config,
        );
    }
    for intent in &module.activate {
        push(
            "activate",
            StepAction::Intent {
                action: Box::new(intent.action.clone()),
                trigger: intent.trigger,
            },
            Phase::Activate,
        );
    }
    steps
}
