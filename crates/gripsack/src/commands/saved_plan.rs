//! Explicit private saved-plan commands. Applying never invokes the evaluator.
use crate::render::{DiagnosticSink, Palette};
use gripsack_exec::{Ctx, Repository, saved_plan::SavedPlan};
use std::{path::Path, process::ExitCode, sync::Arc};

pub fn save(
    repo: &Path,
    host: Option<String>,
    selected: Vec<String>,
    path: &Path,
    palette: Palette,
) -> ExitCode {
    let mut sink = DiagnosticSink::terminal(palette, repo);
    let outcome = match super::eval::eval_repo(repo, host, &mut sink) {
        Ok(outcome) => outcome,
        Err(code) => return code,
    };
    if let Err(code) = super::eval::validated_ir(&outcome, &mut sink) {
        return code;
    }
    let result = (|| -> Result<(), gripsack_exec::ExecError> {
        let absolute = std::path::absolute(path)?;
        if let Ok(relative) = absolute.strip_prefix(outcome.sources.repository_identity())
            && !outcome.sources.capture_policy().excludes(relative)
        {
            return Err(std::io::Error::other("save outside the captured repository (or in an explicitly excluded directory), otherwise saving changes the reviewed source").into());
        }
        let ctx = Ctx {
            home: gripsack_store::gripsack_home(),
            home_dir: Default::default(),
            repository: Repository::evaluated_with_inputs(
                Arc::clone(&outcome.sources),
                outcome.receipt,
                outcome.host_inputs.facts,
                outcome.host_inputs.probes,
            ),
            only: selected,
            host: outcome.host,
            take_over: false,
            take_over_entries: None,
            fetch: outcome.fetch,
            on_progress: None,
        };
        let plan = SavedPlan::capture(&outcome.ir_json, &ctx)?;
        plan.save(path)
    })();
    match result {
        Ok(()) => {
            println!(
                "saved private local execution plan; contains authored configuration, not a portable report; unknown recipe outputs are not reviewed bytes"
            );
            ExitCode::SUCCESS
        }
        Err(error) => failure(error),
    }
}

pub fn apply(repo: &Path, path: &Path, palette: Palette) -> ExitCode {
    let plan = match SavedPlan::load(path) {
        Ok(plan) => plan,
        Err(error) => return failure(error),
    };
    let mut sink = DiagnosticSink::terminal(palette, repo);
    // Capture and authorize current source/policy, but do not evaluate it. The
    // captured IR is the sole declaration input to execution.
    if let Err(error) = plan.preflight(repo, &gripsack_store::gripsack_home()) {
        return failure(error);
    }
    let prepared = match super::prepared::PreparedEvaluation::capture_existing(
        repo,
        Some(plan.host().into()),
        &mut sink,
    ) {
        Ok(prepared) => prepared,
        Err(code) => return code,
    };
    if let Err(code) = prepared.authorize(false, None) {
        return code;
    }
    let result = (|| -> Result<_, gripsack_exec::ExecError> {
        let policy = prepared.policy().digest()?.to_string();
        let facts = serde_json::to_value(gripsack_exec::facts::detect())?;
        let probes = super::probe::reobserve(plan.probes()).map_err(std::io::Error::other)?;
        let fetch = Arc::new(gripsack_fetch::FetchContext::artifacts(
            super::eval::acquisition_limits(&prepared.config.settings),
            Arc::clone(&prepared.provisioning),
            prepared.env.eval.env.clone(),
        ));
        let ctx = Ctx {
            home: prepared.home().to_path_buf(),
            home_dir: Default::default(),
            repository: Repository::evaluated_with_inputs(
                Arc::clone(prepared.sources()),
                plan.evaluation(),
                gripsack_exec::facts::detect().clone(),
                probes.clone(),
            ),
            only: plan.selected().to_vec(),
            host: prepared.host.clone(),
            take_over: false,
            take_over_entries: None,
            fetch,
            on_progress: None,
        };
        plan.apply(&ctx, &policy, &facts, &probes, || {
            super::eval::provision_plugins(&prepared.config.fetchers, &prepared.provisioning)
                .map_err(|_| std::io::Error::other("saved-plan fetcher preparation failed").into())
        })
    })();
    match result {
        Ok(result) => {
            for report in result.reports {
                println!(
                    "{}: {}",
                    report.output.escape_default(),
                    report.summary.escape_default()
                );
            }
            match result.outcome {
                gripsack_exec::Outcome::Applied { generation } => {
                    println!("applied saved plan — generation {generation}")
                }
                gripsack_exec::Outcome::Satisfied { .. } => {
                    println!("saved plan already satisfied")
                }
            }
            ExitCode::SUCCESS
        }
        Err(error) => failure(error),
    }
}

fn failure(error: impl std::fmt::Display) -> ExitCode {
    eprintln!("grip: {}", error.to_string().escape_default());
    ExitCode::FAILURE
}
