//! User-visible step reports (the CLI renders these).

use crate::ctx::Outcome;
use gripsack_ir::Verify;

/// One user-visible line of what a step did — the CLI renders these.
#[derive(Debug, Clone, PartialEq)]
pub struct StepReport {
    pub module: String,
    pub summary: String,
    pub kind: ReportKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportKind {
    Fetched,
    Installed,
    Configured,
    Verified,
    Satisfied,
    Warned,
}

/// The result of an apply: outcome + the reports for the CLI.
#[derive(Debug)]
pub struct ApplyResult {
    pub outcome: Outcome,
    pub reports: Vec<StepReport>,
}

pub(crate) fn describe_fetch(spec: &gripsack_ir::FetchSpec) -> String {
    use gripsack_ir::FetchSpec as F;
    match spec {
        F::GithubRelease { repo, asset, .. } => format!("github-release {repo} · {asset}"),
        F::Tarball { url, .. } => format!("tarball {url}"),
        F::Git { url, rev } => format!("git {url} @ {rev}"),
        F::File { path } => format!("file {path}"),
        F::Plugin { name, .. } => format!("plugin gripfetch-{name}"),
        F::Brew { formula, .. } => format!("brew {formula}"),
        F::Pixi { package, .. } => format!("pixi {package}"),
    }
}

pub(crate) fn describe_verify(verify: &Verify, version: Option<&str>) -> String {
    let sub = |path: &str| match version {
        Some(v) => path.replace("{version}", v),
        None => path.to_string(),
    };
    match verify {
        Verify::BinaryRuns { path, .. } => format!("verified {} runs", sub(path)),
        Verify::FileExists { path } => format!("verified {path} exists"),
        Verify::Shell { .. } => "verified (shell check)".to_string(),
        Verify::FileDeployed { path } => format!("verified {path} deployed"),
    }
}

/// One line of an update report.
#[derive(Debug, Clone, PartialEq)]
pub struct UpdateReport {
    pub module: String,
    pub status: UpdateStatus,
}

#[derive(Debug, Clone, PartialEq)]
pub enum UpdateStatus {
    Unchanged,
    /// New or bumped pin — apply to deploy it.
    Bumped {
        old: Option<String>,
        new: String,
    },
    /// Resolution for this fetch kind is not supported yet (github_release, git).
    Skipped,
}
