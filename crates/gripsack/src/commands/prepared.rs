//! One captured source/runtime/policy owner spans all eval rounds.
mod configuration;
mod provenance;
mod runtime;

use crate::render::DiagnosticSink;
use gripsack_exec::{
    source_bundle::{SourceBundle, SourceCapturePolicy},
    trust::{EvaluationPolicy, GitProvenance},
};
use gripsack_host::process::{Limits, OperatorEnvironment, SelectedProgram};
use gripsack_ir::HostName;
use std::{
    io::{self},
    path::{Path, PathBuf},
    process::ExitCode,
    sync::Arc,
    time::Instant,
};

pub(super) struct PreparedEvaluation {
    sources: Arc<SourceBundle>,
    runtime: SelectedProgram,
    environment: OperatorEnvironment,
    policy: EvaluationPolicy,
    provenance: GitProvenance,
    home: PathBuf,
    pub(super) env: gripsack_config::EnvConfig,
    pub(super) config: gripsack_config::Config,
    pub(super) host: HostName,
    pub(super) provisioning: Arc<gripsack_fetch::FetchContext>,
    runtime_access: gripsack_host::process::RuntimeAccess,
    evaluator_cache: PathBuf,
    evaluator_tmp: PathBuf,
}

/// The captured evaluation inputs, borrowed for the evaluator run.
pub(super) struct ApprovedEvaluation<'a> {
    prepared: &'a PreparedEvaluation,
}

impl PreparedEvaluation {
    pub(super) fn capture(
        repo: &Path,
        host: Option<String>,
        sink: &mut DiagnosticSink,
    ) -> Result<Self, ExitCode> {
        Self::capture_with(repo, host, sink, false)
    }

    /// Saved execution may inspect existing evaluator identities but never
    /// provision a runtime/frontend or execute the evaluator.
    pub(super) fn capture_existing(
        repo: &Path,
        host: Option<String>,
        sink: &mut DiagnosticSink,
    ) -> Result<Self, ExitCode> {
        Self::capture_with(repo, host, sink, true)
    }

    fn capture_with(
        repo: &Path,
        host: Option<String>,
        sink: &mut DiagnosticSink,
        existing_only: bool,
    ) -> Result<Self, ExitCode> {
        // Preserve host admission before provisioning or host-derived IO. This
        // preliminary live read grants nothing; all effective config is read
        // again from the captured tree below, before approval.
        let explicit_host = host
            .map(HostName::parse)
            .transpose()
            .map_err(|diagnostic| {
                sink.report(&[diagnostic]);
                ExitCode::FAILURE
            })?;
        let mut initial = configuration::Configuration::read(repo, None, sink)?;
        if explicit_host.is_none()
            && let Some(host) = &initial.env.env.default_host
        {
            HostName::parse(host.clone()).map_err(|diagnostic| {
                sink.report(&[diagnostic]);
                ExitCode::FAILURE
            })?;
        }
        let capture_policy =
            SourceCapturePolicy::new(std::mem::take(&mut initial.env.capture.exclude))
                .map_err(operational)?;
        let environment = OperatorEnvironment::capture().map_err(operational)?;
        let home =
            std::path::absolute(gripsack_host::store::gripsack_home()).map_err(operational)?;
        let frontend = if existing_only {
            let path = home
                .join("frontend")
                .join(format!("ts-{}", env!("CARGO_PKG_VERSION")));
            if !path.join(".complete").is_file() {
                return Err(operational(io::Error::other(
                    "saved apply requires the captured frontend; run explicit live planning",
                )));
            }
            path
        } else {
            gripsack_exec::ensure_ts_frontend(&home, env!("CARGO_PKG_VERSION"))
                .map_err(operational)?
                .ok_or_else(|| {
                    operational(io::Error::other(
                        "this binary has no embedded TypeScript frontend",
                    ))
                })?
        };
        let pin_relative = Path::new("node_modules/@gripsack/core");
        // An editor-only SDK exclusion must precede metadata/canonicalization:
        // it is not an alternative way to admit the external package root.
        let pin = if capture_policy.excludes(pin_relative) {
            None
        } else {
            let pin_name = repo.join(pin_relative);
            match std::fs::symlink_metadata(&pin_name) {
                Ok(_) => Some(pin_name.canonicalize().map_err(operational)?),
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(operational(error)),
            }
        };
        let sources = Arc::new(
            SourceBundle::capture(repo, &frontend, pin.as_deref(), &home, capture_policy)
                .map_err(operational)?,
        );
        sink.bind_captured_source(Arc::clone(&sources));
        let captured =
            configuration::Configuration::read(sources.repository(), Some(&sources), sink)?;
        initial.require_same(&captured).map_err(operational)?;
        let env = captured.env;
        let host = match explicit_host {
            Some(host) => host,
            None => HostName::parse(
                env.env
                    .default_host
                    .clone()
                    .unwrap_or_else(super::default_host),
            )
            .map_err(|diagnostic| {
                sink.report(&[diagnostic]);
                ExitCode::FAILURE
            })?,
        };
        let user = std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| gripsack_config::load_user(&home.join(".config/gripsack/config.toml")))
            .transpose()
            .map_err(|diagnostics| {
                sink.report(&diagnostics);
                ExitCode::FAILURE
            })?;
        let mut config = gripsack_config::merge(user.as_ref(), &env);
        let provisioning = Arc::new(gripsack_fetch::FetchContext::new(
            super::eval::acquisition_limits(&config.settings),
        ));
        // Only the core-selected runtime is provisioned before approval. Repo
        // fetcher provisioning belongs to the later authorized path.
        let deno = if existing_only {
            gripsack_exec::saved_plan::SavedPlan::existing_runtime(&home)
        } else {
            gripsack_exec::ensure_deno(&home, &provisioning)
        }
        .map_err(operational)?;
        let runtime_access =
            runtime::access(&sources, &environment, &deno, &home).map_err(operational)?;
        let evaluator_cache = home.join("deno-cache");
        let evaluator_tmp = home.join("eval-tmp");
        if !existing_only {
            std::fs::create_dir_all(&evaluator_cache).map_err(operational)?;
            std::fs::create_dir_all(&evaluator_tmp).map_err(operational)?;
        }
        let environment = environment
            .with_evaluator_cache(&evaluator_cache)
            .and_then(|environment| environment.with_evaluator_temp(&evaluator_tmp))
            .map_err(operational)?;
        let limits = Limits::default();
        let deadline = Instant::now()
            .checked_add(limits.timeout)
            .ok_or_else(|| operational(io::Error::other("runtime selection deadline overflow")))?;
        let runtime = SelectedProgram::select(
            &environment,
            &deno,
            Some(runtime_access.primary_digest().map_err(operational)?),
            deadline,
        )
        .map_err(operational)?;
        runtime_access
            .require_selected_identity(&runtime.identity())
            .map_err(operational)?;
        let evaluator_cache = sources
            .admit_evaluator_root(&evaluator_cache)
            .map_err(operational)?;
        let evaluator_tmp = sources
            .admit_evaluator_root(&evaluator_tmp)
            .map_err(operational)?;
        let policy = EvaluationPolicy::capture(
            &sources,
            runtime.identity(),
            &(&config, &runtime_access),
            limits,
            super::probe::PROBE_ROUNDS,
            super::probe::INPUT_DOCUMENT_BYTES,
        )
        .map_err(operational)?;
        // The policy hashes logical declarations. Execution binds their local
        // paths to the approved snapshot rather than rereading the worktree.
        for fetcher in config.fetchers.values_mut() {
            for path in [&mut fetcher.plugin, &mut fetcher.path]
                .into_iter()
                .flatten()
            {
                if let std::borrow::Cow::Owned(mapped) =
                    sources.native_path(path).map_err(operational)?
                {
                    *path = mapped;
                }
            }
        }
        let provenance = provenance::capture(sources.repository_identity(), &environment);
        Ok(Self {
            sources,
            runtime,
            environment,
            policy,
            provenance,
            home,
            env,
            config,
            host,
            provisioning,
            runtime_access,
            evaluator_cache,
            evaluator_tmp,
        })
    }

    pub(super) fn sources(&self) -> &Arc<SourceBundle> {
        &self.sources
    }
    pub(super) fn policy(&self) -> &EvaluationPolicy {
        &self.policy
    }
    pub(super) fn provenance(&self) -> &GitProvenance {
        &self.provenance
    }
    pub(super) fn home(&self) -> &Path {
        &self.home
    }

    /// Running grip on a repo is consent (#409): there is no approval gate.
    /// Remote origins are confirmed once, after evaluation, by `remote`.
    pub(super) fn authorize(&self) -> ApprovedEvaluation<'_> {
        ApprovedEvaluation { prepared: self }
    }

    pub(super) fn map_diagnostics(&self, diagnostics: &mut [gripsack_ir::Diagnostic]) {
        map_diagnostics(&self.sources, diagnostics);
    }
}

impl ApprovedEvaluation<'_> {
    pub(super) fn sources(&self) -> &SourceBundle {
        &self.prepared.sources
    }
    pub(super) fn runtime(&self) -> &SelectedProgram {
        &self.prepared.runtime
    }
    pub(super) fn environment(&self) -> &OperatorEnvironment {
        &self.prepared.environment
    }
    pub(super) fn policy(&self) -> &EvaluationPolicy {
        &self.prepared.policy
    }
    pub(super) fn runtime_access(&self) -> &gripsack_host::process::RuntimeAccess {
        &self.prepared.runtime_access
    }
    pub(super) fn evaluator_cache(&self) -> &Path {
        &self.prepared.evaluator_cache
    }
    pub(super) fn evaluator_tmp(&self) -> &Path {
        &self.prepared.evaluator_tmp
    }

    pub(super) fn map_diagnostics(&self, diagnostics: &mut [gripsack_ir::Diagnostic]) {
        self.prepared.map_diagnostics(diagnostics);
    }
}

fn map_diagnostics(sources: &SourceBundle, diagnostics: &mut [gripsack_ir::Diagnostic]) {
    fn map(sources: &SourceBundle, text: &mut String) {
        if let std::borrow::Cow::Owned(mapped) = sources.logical_text(text) {
            *text = mapped;
        }
    }
    for diagnostic in diagnostics {
        map(sources, &mut diagnostic.message);
        if let Some(help) = &mut diagnostic.help {
            map(sources, help);
        }
        for label in &mut diagnostic.labels {
            if let Some(span) = &mut label.span {
                map(sources, &mut span.file);
            }
            map(sources, &mut label.note);
        }
    }
}

pub(super) fn operational(error: impl std::fmt::Display) -> ExitCode {
    eprintln!(
        "grip: {}",
        gripsack_host::process::terminal::tame(error.to_string())
    );
    ExitCode::FAILURE
}
