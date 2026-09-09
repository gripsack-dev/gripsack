//! Layout evidence only: never execute recipes or verification programs.
use crate::ctx::ExecError;
use gripsack_ir::{Ownership, Verify, prepared::PreparedModule};
use std::path::Path;

#[derive(Debug, Default)]
pub struct LayoutEvidence {
    pub checked_paths: usize,
    pub deferred: Vec<DeferredLayoutCheck>,
}

#[derive(Debug)]
pub enum DeferredLayoutCheck {
    RecipeOutput(String),
    RuntimeVerification(&'static str),
    UnavailableArtifact,
}

pub(crate) enum PayloadStage<'a> {
    AcquiredSource(&'a Path),
    PublishedArtifact(&'a Path),
}

impl LayoutEvidence {
    pub fn summary(&self) -> Option<String> {
        if self.checked_paths == 0 && self.deferred.is_empty() {
            return None;
        }
        let mut text = format!("layout: {} paths checked", self.checked_paths);
        for deferred in &self.deferred {
            match deferred {
                DeferredLayoutCheck::RecipeOutput(path) => {
                    text.push_str(&format!("; {path} deferred (recipe output)"))
                }
                DeferredLayoutCheck::RuntimeVerification(kind) => {
                    text.push_str(&format!("; {kind} deferred (runtime verification)"))
                }
                DeferredLayoutCheck::UnavailableArtifact => {
                    text.push_str("; deferred (matching artifact unavailable, no fetch performed)")
                }
            }
        }
        Some(text)
    }
}

pub(crate) fn inspect(
    name: &str,
    plan: &PreparedModule,
    payload: PayloadStage<'_>,
    version: Option<&str>,
) -> Result<LayoutEvidence, ExecError> {
    let recipe_pending = plan.has_recipe() && matches!(payload, PayloadStage::AcquiredSource(_));
    let stage = match payload {
        PayloadStage::AcquiredSource(path) | PayloadStage::PublishedArtifact(path) => path,
    };
    let mut evidence = LayoutEvidence::default();
    let fail = |pattern: &str, detail: String| ExecError::Step {
        module: name.into(),
        step: "preflight".into(),
        detail: format!(
            "{pattern:?} (locked version {}): {detail}",
            version.unwrap_or("unavailable")
        ),
    };
    let mut check_path = |pattern: &str, regular: bool| -> Result<(), ExecError> {
        let source = super::payload_source(stage, pattern, version)
            .map_err(|e| fail(pattern, e.to_string()))?;
        if recipe_pending {
            evidence
                .deferred
                .push(DeferredLayoutCheck::RecipeOutput(source.relative));
            return Ok(());
        }
        let metadata = std::fs::metadata(&source.path).map_err(|error| {
            let top_level = match std::fs::read_dir(stage) {
                Ok(entries) => {
                    let names: std::io::Result<Vec<_>> = entries
                        .map(|entry| {
                            entry.map(|entry| entry.file_name().to_string_lossy().into_owned())
                        })
                        .collect();
                    match names {
                        Ok(mut names) => {
                            names.sort();
                            names.join(", ")
                        }
                        Err(error) => format!("<unavailable: {error}>"),
                    }
                }
                Err(error) => format!("<unavailable: {error}>"),
            };
            let hint =
                if pattern.contains("{version}") && version.is_some_and(|v| v.starts_with('v')) {
                    " — if the payload omits the tag's v prefix, use {version.bare}"
                } else {
                    ""
                };
            fail(
                pattern,
                format!(
                    "cannot inspect {}: {error}; payload top-level: {top_level}{hint}",
                    source.relative
                ),
            )
        })?;
        if regular && !metadata.is_file() {
            return Err(fail(
                pattern,
                format!("{} must be a regular file", source.relative),
            ));
        }
        if !metadata.is_file() && !metadata.is_dir() {
            return Err(fail(
                pattern,
                format!("{} is not a regular payload entry", source.relative),
            ));
        }
        evidence.checked_paths += 1;
        Ok(())
    };
    for entry in plan.entries() {
        check_path(&entry.from, entry.mode != Ownership::Owned).map_err(|error| {
            if let Some(span) = &entry.span {
                ExecError::Step {
                    module: name.into(),
                    step: "preflight".into(),
                    detail: format!("{}:{}: {error}", span.file, span.line),
                }
            } else {
                error
            }
        })?;
    }
    // Keep the mutable path-check borrow local before recording opaque checks.
    let mut runtime = Vec::new();
    for verify in plan.checks() {
        match verify {
            Verify::FileExists { path } => check_path(path, false)?,
            Verify::BinaryRuns { path, .. } => {
                check_path(path, true)?;
                runtime.push(DeferredLayoutCheck::RuntimeVerification("binary execution"));
            }
            Verify::Shell { .. } => {
                runtime.push(DeferredLayoutCheck::RuntimeVerification("shell check"))
            }
            Verify::FileDeployed { .. } => runtime.push(DeferredLayoutCheck::RuntimeVerification(
                "deployed destination",
            )),
        }
    }
    evidence.deferred.extend(runtime);
    Ok(evidence)
}

/// Existing matching artifacts only. No resolver, download, build or publication.
pub fn inspect_known(
    ir: &gripsack_ir::Ir,
    repo: &Path,
    host: &str,
) -> Result<Vec<(String, LayoutEvidence)>, ExecError> {
    let lock = match crate::lockfile::read(repo, host) {
        crate::lockfile::LockRead::Parsed(lock) => lock,
        crate::lockfile::LockRead::Missing => Default::default(),
        crate::lockfile::LockRead::Corrupt(detail) => {
            return Err(ExecError::Step {
                module: "*".into(),
                step: "lockfile".into(),
                detail,
            });
        }
    };
    let plans = crate::expand::expand_all(&ir.modules)?;
    let recipes =
        crate::resolve::RecipeGraph::new(ir, repo, &plans, ir.modules.keys().map(String::as_str))?;
    let home = gripsack_store::gripsack_home();
    let mut reports = Vec::new();
    for (name, plan) in &plans {
        if plan.entries().next().is_none() && plan.checks().next().is_none() {
            continue;
        }
        let locked = lock.modules.get(name);
        let identity = crate::identity::resolve(crate::identity::IdentityInputs {
            name,
            recipes: &recipes,
            plan,
            home: &home,
            repo,
            locked,
            lock: &lock,
        })?;
        let evidence = if identity.present {
            inspect(
                name,
                plan,
                PayloadStage::PublishedArtifact(&identity.store_path),
                locked
                    .and_then(|entry| entry.resolved.as_ref())
                    .and_then(|pin| pin.version.as_deref()),
            )?
        } else {
            LayoutEvidence {
                checked_paths: 0,
                deferred: vec![DeferredLayoutCheck::UnavailableArtifact],
            }
        };
        reports.push((name.clone(), evidence));
    }
    Ok(reports)
}
