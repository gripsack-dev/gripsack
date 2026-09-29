//! User-visible step reports (the CLI renders these).

use crate::ctx::Outcome;
use gripsack_ir::Verify;
use gripsack_policy::update_survey::{SurveyProgress, UpdateDisposition};
pub use gripsack_policy::update_survey::{UpdateCheckOutcome, UpdateSummary};

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

/// A returned survey accounts for every admitted selected entry.
#[derive(Debug)]
pub struct UpdateSurvey {
    reports: Vec<UpdateReport>,
    summary: UpdateSummary,
}

impl UpdateSurvey {
    pub fn reports(&self) -> &[UpdateReport] {
        &self.reports
    }

    pub fn summary(&self) -> &UpdateSummary {
        &self.summary
    }
}

pub(crate) struct SurveyReports {
    reports: Vec<UpdateReport>,
    progress: SurveyProgress,
}

impl SurveyReports {
    pub(crate) fn new(selected: usize) -> Self {
        Self {
            reports: Vec::new(),
            progress: SurveyProgress::new(selected),
        }
    }

    pub(crate) fn push(&mut self, report: UpdateReport) -> Result<(), crate::ExecError> {
        let disposition = match &report.status {
            UpdateStatus::Unchanged => UpdateDisposition::Unchanged,
            UpdateStatus::Bumped { .. } => UpdateDisposition::Changed,
            UpdateStatus::Skipped { .. } => UpdateDisposition::Skipped,
            UpdateStatus::Failed { .. } => UpdateDisposition::Failed,
        };
        if !self.progress.record(self.reports.len(), disposition) {
            return Err(survey_accounting_error());
        }
        self.reports.push(report);
        Ok(())
    }

    pub(crate) fn finish(self) -> Result<UpdateSurvey, crate::ExecError> {
        let summary = self
            .progress
            .finish(self.reports.len())
            .ok_or_else(survey_accounting_error)?;
        Ok(UpdateSurvey {
            reports: self.reports,
            summary,
        })
    }
}

fn survey_accounting_error() -> crate::ExecError {
    crate::ExecError::Step {
        module: "*".into(),
        step: "survey".into(),
        detail: "update report accounting does not match the selected entries".into(),
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
                    let mut reports = SurveyReports::new(kinds.len());
                    for (index, kind) in kinds.into_iter().enumerate() {
                        reports
                            .push(UpdateReport {
                                module: format!("fixture-{index}"),
                                layout: Default::default(),
                                status: match kind {
                                    0 => UpdateStatus::Unchanged,
                                    1 => UpdateStatus::Bumped {
                                        old: None,
                                        new: "v2".into(),
                                    },
                                    2 => UpdateStatus::Failed {
                                        error: Box::new(crate::ctx::ExecError::Step {
                                            module: format!("fixture-{index}"),
                                            step: "resolve".into(),
                                            detail: "unavailable".into(),
                                        }),
                                    },
                                    _ => UpdateStatus::Skipped {
                                        reason: "no fetch source",
                                    },
                                },
                            })
                            .unwrap();
                    }
                    let expected = if kinds.contains(&2) {
                        UpdateCheckOutcome::Incomplete
                    } else if kinds.contains(&1) {
                        UpdateCheckOutcome::ChangesAvailable
                    } else {
                        UpdateCheckOutcome::Current
                    };
                    let survey = reports.finish().unwrap();
                    assert_eq!(survey.summary().outcome(), expected);
                    assert_eq!(survey.summary().selected(), kinds.len());
                    assert_eq!(
                        survey.summary().unchanged(),
                        kinds.iter().filter(|&&kind| kind == 0).count()
                    );
                    assert_eq!(
                        survey.summary().changed(),
                        kinds.iter().filter(|&&kind| kind == 1).count()
                    );
                    assert_eq!(
                        survey.summary().failed(),
                        kinds.iter().filter(|&&kind| kind == 2).count()
                    );
                    assert_eq!(
                        survey.summary().skipped(),
                        kinds.iter().filter(|&&kind| kind == 3).count()
                    );
                }
            }
        }
        assert_eq!(
            SurveyReports::new(0).finish().unwrap().summary().outcome(),
            UpdateCheckOutcome::Current
        );
    }
}
