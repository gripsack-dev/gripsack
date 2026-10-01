//! One captured source/runtime/policy owner spans approval and all eval rounds.
mod provenance;

use crate::render::DiagnosticSink;
use gripsack_ir::HostName;
use gripsack_process::{Limits, OperatorEnvironment, SelectedProgram};
use gripsack_store::{
    source_bundle::SourceBundle,
    trust::{self, ApprovalStatus, EvaluationPolicy, GitProvenance},
};
use std::{
    io::{self, IsTerminal, Write},
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
    runtime_roots: Vec<PathBuf>,
    evaluator_cache: PathBuf,
    evaluator_tmp: PathBuf,
}

/// Only a completed source/policy gate constructs this borrowing authority.
pub(super) struct ApprovedEvaluation<'a> {
    prepared: &'a PreparedEvaluation,
}

impl PreparedEvaluation {
    pub(super) fn capture(
        repo: &Path,
        host: Option<String>,
        sink: &mut DiagnosticSink,
    ) -> Result<Self, ExitCode> {
        reject_blanket_bypass()?;
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
        let initial = load_env(repo, None, sink)?;
        if explicit_host.is_none()
            && let Some(host) = &initial.env.default_host
        {
            HostName::parse(host.clone()).map_err(|diagnostic| {
                sink.report(&[diagnostic]);
                ExitCode::FAILURE
            })?;
        }
        drop(initial);
        let environment = OperatorEnvironment::capture().map_err(operational)?;
        let home = std::path::absolute(gripsack_store::gripsack_home()).map_err(operational)?;
        let frontend = gripsack_exec::ensure_ts_frontend(&home, env!("CARGO_PKG_VERSION"))
            .map_err(operational)?
            .ok_or_else(|| {
                operational(io::Error::other(
                    "this binary has no embedded TypeScript frontend",
                ))
            })?;
        let pin_name = repo.join("node_modules/@gripsack/core");
        let pin = match std::fs::symlink_metadata(&pin_name) {
            Ok(_) => Some(pin_name.canonicalize().map_err(operational)?),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(operational(error)),
        };
        let sources = Arc::new(
            SourceBundle::capture(repo, &frontend, pin.as_deref(), &home).map_err(operational)?,
        );
        sink.bind_captured_source(Arc::clone(&sources));
        let mut env = load_env(sources.repository(), Some(&sources), sink)?;
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
        // fetcher/linter provisioning belongs to the later authorized path.
        let deno = gripsack_exec::ensure_deno(&home, &provisioning).map_err(operational)?;
        let evaluator_cache = home.join("deno-cache");
        let evaluator_tmp = home.join("eval-tmp");
        std::fs::create_dir_all(&evaluator_cache).map_err(operational)?;
        std::fs::create_dir_all(&evaluator_tmp).map_err(operational)?;
        let environment = environment
            .with_evaluator_cache(&evaluator_cache)
            .and_then(|environment| environment.with_evaluator_temp(&evaluator_tmp))
            .map_err(operational)?;
        let limits = Limits::default();
        let deadline = Instant::now()
            .checked_add(limits.timeout)
            .ok_or_else(|| operational(io::Error::other("runtime selection deadline overflow")))?;
        let runtime =
            SelectedProgram::select(&environment, &deno, None, deadline).map_err(operational)?;
        let runtime_roots =
            gripsack_process::runtime_read_roots(&environment, &deno).map_err(operational)?;
        let policy = EvaluationPolicy::capture(
            &sources,
            runtime.identity(),
            &(&config, &env.linters),
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
        for linter in env.linters.values_mut() {
            if let Some(path) = &mut linter.path
                && let std::borrow::Cow::Owned(mapped) =
                    sources.native_path(path).map_err(operational)?
            {
                *path = mapped;
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
            runtime_roots,
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

    pub(super) fn authorize(&self, interactive: bool) -> Result<ApprovedEvaluation<'_>, ExitCode> {
        let status = trust::status(&self.home, &self.sources, &self.policy).map_err(operational)?;
        if status == ApprovalStatus::Approved {
            return Ok(ApprovedEvaluation { prepared: self });
        }
        let policy = self.policy.digest().map_err(operational)?;
        if !interactive || !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            let migration = if status == ApprovalStatus::Legacy {
                "legacy path-only approval requires renewal"
            } else {
                "captured source/policy is not approved"
            };
            eprintln!(
                "grip: {migration} for {}",
                gripsack_process::terminal::tame(
                    self.sources.repository_identity().display().to_string()
                )
            );
            eprintln!(
                "hint: inspect with `grip trust inspect --json`, then approve with `grip trust add --bundle {} --policy {policy}`",
                self.sources.digest()
            );
            return Err(ExitCode::FAILURE);
        }
        self.show_approval()?;
        let mut output = io::stdout().lock();
        write!(
            output,
            "approve these captured bytes and this policy? [y/N] "
        )
        .map_err(operational)?;
        output.flush().map_err(operational)?;
        let mut answer = String::new();
        io::stdin().read_line(&mut answer).map_err(operational)?;
        if !answer.trim().eq_ignore_ascii_case("y") {
            eprintln!("grip: source approval declined — no repository code was evaluated");
            return Err(ExitCode::FAILURE);
        }
        trust::approve(
            &self.home,
            &self.sources,
            &self.policy,
            self.sources.digest(),
            policy,
            &self.provenance,
        )
        .map_err(operational)?;
        Ok(ApprovedEvaluation { prepared: self })
    }

    pub(super) fn show_approval(&self) -> Result<(), ExitCode> {
        let inspection =
            trust::inspect(&self.home, &self.sources, &self.policy).map_err(operational)?;
        println!(
            "repository: {}",
            gripsack_process::terminal::tame(
                self.sources.repository_identity().display().to_string()
            )
        );
        println!("bundle: {}", self.sources.digest());
        println!("policy: {}", self.policy.digest().map_err(operational)?);
        println!(
            "runtime: {} ({:?})",
            self.policy.runtime.executable_sha256, self.policy.runtime.byte_binding
        );
        println!(
            "evaluator: captured source and one immutable input file; no env, network, subprocess, FFI or system grants"
        );
        println!(
            "native policy: executable/file probes; declared fetch/build/verify/hooks only through explicit commands"
        );
        println!(
            "native configuration: {}",
            self.policy.native_configuration_sha256
        );
        println!(
            "inventory: {} objects, {} changes",
            self.sources.inventory().entries().len(),
            inspection.changes.len()
        );
        for change in inspection.changes.iter().take(20) {
            println!(
                "  {:?} {}",
                change.change,
                gripsack_process::terminal::tame(change.path.clone())
            );
        }
        if inspection.changes.len() > 20 {
            println!("  use --json to inspect every inventory change");
        }
        for excluded in self.sources.inventory().exclusions() {
            println!(
                "  unavailable: {}",
                gripsack_process::terminal::tame(excluded.clone())
            );
        }
        Ok(())
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
    pub(super) fn runtime_roots(&self) -> &[PathBuf] {
        &self.prepared.runtime_roots
    }
    pub(super) fn evaluator_cache(&self) -> &Path {
        &self.prepared.evaluator_cache
    }
    pub(super) fn evaluator_tmp(&self) -> &Path {
        &self.prepared.evaluator_tmp
    }

    /// The provisioned evaluator runtime tree. Executing a binary under
    /// Landlock requires reading it, and an operator-selected script runtime
    /// commonly execs the pinned runtime gripsack provisioned here.
    pub(super) fn runtime_home(&self) -> PathBuf {
        self.prepared.home.join("tools")
    }
    pub(super) fn map_diagnostics(&self, diagnostics: &mut [gripsack_ir::Diagnostic]) {
        self.prepared.map_diagnostics(diagnostics);
    }
}

fn load_env(
    repo: &Path,
    sources: Option<&SourceBundle>,
    sink: &mut DiagnosticSink,
) -> Result<gripsack_config::EnvConfig, ExitCode> {
    let path = repo.join("env.toml");
    match std::fs::symlink_metadata(&path) {
        Ok(_) => gripsack_config::load_env(&path).map_err(|mut diagnostics| {
            if let Some(sources) = sources {
                map_diagnostics(sources, &mut diagnostics);
            }
            sink.report(&diagnostics);
            ExitCode::FAILURE
        }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            match std::fs::metadata(repo.join("gripsack.ts")) {
                Ok(metadata) if metadata.is_file() => Ok(gripsack_config::EnvConfig::default()),
                Ok(_) => Err(operational(io::Error::other(
                    "gripsack.ts is not a regular workspace source",
                ))),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Err(operational(
                    io::Error::other("no env.toml or gripsack.ts in the repository"),
                )),
                Err(error) => Err(operational(error)),
            }
        }
        Err(error) => Err(operational(error)),
    }
}

pub(super) fn map_diagnostics(sources: &SourceBundle, diagnostics: &mut [gripsack_ir::Diagnostic]) {
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

pub(super) fn reject_blanket_bypass() -> Result<(), ExitCode> {
    if std::env::var_os("GRIPSACK_TRUST_ALL").is_some_and(|value| value == "1") {
        eprintln!(
            "grip: GRIPSACK_TRUST_ALL no longer grants approval; unset it, inspect the captured bundle and use `grip trust add --bundle <digest> --policy <digest>`"
        );
        return Err(ExitCode::FAILURE);
    }
    Ok(())
}

pub(super) fn operational(error: impl std::fmt::Display) -> ExitCode {
    eprintln!(
        "grip: {}",
        gripsack_process::terminal::tame(error.to_string())
    );
    ExitCode::FAILURE
}
