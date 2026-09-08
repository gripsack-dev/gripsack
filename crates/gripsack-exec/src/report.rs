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

pub fn describe_fetch(spec: &gripsack_ir::FetchSpec) -> String {
    use gripsack_ir::FetchSpec as F;
    match spec {
        F::GithubRelease { repo, asset, .. } => format!("github-release {repo} · {asset}"),
        F::Tarball { url, .. } => format!("tarball {url}"),
        F::Git { url, rev } => format!("git {url} @ {}", rev.as_deref().unwrap_or("HEAD (float)")),
        F::File { path } => format!("file {path}"),
        F::Plugin { name, .. } => format!("plugin gripfetch-{name}"),
        F::Brew { formula, .. } => format!("brew {formula}"),
        F::Pixi { package, .. } => format!("pixi {package}"),
    }
}

pub(crate) fn describe_verify(
    verify: &Verify,
    version: Option<&str>,
) -> Result<String, gripsack_fetch::PlaceholderError> {
    let sub = |path: &str| gripsack_fetch::placeholders::payload_path(path, version);
    Ok(match verify {
        Verify::BinaryRuns { path, .. } => format!("verified {} runs", sub(path)?),
        Verify::FileExists { path } => format!("verified {} exists", sub(path)?),
        Verify::Shell { .. } => "verified (shell check)".to_string(),
        Verify::FileDeployed { path } => format!("verified {path} deployed"),
    })
}

/// One line of an update report.
#[derive(Debug)]
pub struct UpdateReport {
    pub module: String,
    pub status: UpdateStatus,
    pub layout: crate::source::preflight::LayoutEvidence,
}

#[derive(Debug)]
pub enum UpdateStatus {
    Unchanged,
    /// New or bumped pin — apply to deploy it.
    Bumped {
        old: Option<String>,
        new: String,
    },
    /// Resolution is not applicable — the reason says why (an inline
    /// git rev IS the pin; anything else the core can't resolve yet).
    Skipped {
        reason: &'static str,
    },
    Failed {
        error: Box<crate::ctx::ExecError>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateCheckOutcome {
    Current,
    ChangesAvailable,
    Incomplete,
}

#[derive(Debug, Default)]
pub struct UpdateSummary {
    pub unchanged: usize,
    pub changed: usize,
    pub skipped: usize,
    pub failed: usize,
}

impl UpdateSummary {
    pub fn from_reports(reports: &[UpdateReport]) -> Self {
        let mut summary = Self::default();
        for report in reports {
            match report.status {
                UpdateStatus::Unchanged => summary.unchanged += 1,
                UpdateStatus::Bumped { .. } => summary.changed += 1,
                UpdateStatus::Skipped { .. } => summary.skipped += 1,
                UpdateStatus::Failed { .. } => summary.failed += 1,
            }
        }
        summary
    }

    pub fn outcome(&self) -> UpdateCheckOutcome {
        if self.failed != 0 {
            UpdateCheckOutcome::Incomplete
        } else if self.changed != 0 {
            UpdateCheckOutcome::ChangesAvailable
        } else {
            UpdateCheckOutcome::Current
        }
    }
}

#[cfg(test)]
mod update_model {
    use super::*;
    #[test]
    fn every_survey_order_preserves_error_over_change_precedence() {
        for first in 0..4 {
            for second in 0..4 {
                for third in 0..4 {
                    let kinds = [first, second, third];
                    let reports = kinds
                        .into_iter()
                        .map(|kind| UpdateReport {
                            module: "fixture".into(),
                            layout: Default::default(),
                            status: match kind {
                                0 => UpdateStatus::Unchanged,
                                1 => UpdateStatus::Bumped {
                                    old: None,
                                    new: "v2".into(),
                                },
                                2 => UpdateStatus::Failed {
                                    error: Box::new(crate::ctx::ExecError::Step {
                                        module: "fixture".into(),
                                        step: "resolve".into(),
                                        detail: "unavailable".into(),
                                    }),
                                },
                                _ => UpdateStatus::Skipped {
                                    reason: "no fetch source",
                                },
                            },
                        })
                        .collect::<Vec<_>>();
                    let expected = if kinds.contains(&2) {
                        UpdateCheckOutcome::Incomplete
                    } else if kinds.contains(&1) {
                        UpdateCheckOutcome::ChangesAvailable
                    } else {
                        UpdateCheckOutcome::Current
                    };
                    assert_eq!(UpdateSummary::from_reports(&reports).outcome(), expected);
                }
            }
        }
        assert_eq!(
            UpdateSummary::from_reports(&[]).outcome(),
            UpdateCheckOutcome::Current
        );
    }
}
