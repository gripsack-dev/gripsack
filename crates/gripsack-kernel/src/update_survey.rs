//! Complete-only update accounting and the lock publication decision
//! (plan/0056 F2 step 10).
//!
//! `grip update` surveys every selected lock entry and reports each one
//! exactly once, in order. The survey admits a report only at its own
//! position and only while entries remain; any other report poisons the
//! survey for good. A summary exists only for a survey that accounted for
//! every selected entry, and the lock is published only from such a summary,
//! in publish mode, with no failed entry and at least one change.
//!
//! Proved (proofs/Kernel/UpdateSurvey.lean): `record` admits exactly the next
//! report of a valid, unfinished survey and counts it under its disposition;
//! a rejected report poisons the survey; `finish` yields a summary exactly
//! for a valid survey that counted every selected entry; a summary's counts
//! add up to the selection; a failure outranks a change in the outcome; and
//! the lock is published only in publish mode, with no failure and a change.

/// What `grip update` does with a complete survey.
pub enum UpdateMode {
    /// Write the new lock.
    Publish,
    /// Report only (`grip update --check`).
    Check,
}

/// How one selected entry was settled.
pub enum UpdateDisposition {
    Unchanged,
    Changed,
    Skipped,
    Failed,
}

/// The survey's verdict (`grip update --check` exit status).
pub enum UpdateCheckOutcome {
    Current,
    ChangesAvailable,
    Incomplete,
}

/// A survey in progress: how many entries were selected and how the reported
/// ones were settled. `invalid` is set by the first out-of-order report and
/// never cleared.
pub struct SurveyProgress {
    selected: usize,
    unchanged: usize,
    changed: usize,
    skipped: usize,
    failed: usize,
    invalid: bool,
}

impl SurveyProgress {
    pub fn new(selected: usize) -> Self {
        Self {
            selected,
            unchanged: 0,
            changed: 0,
            skipped: 0,
            failed: 0,
            invalid: false,
        }
    }

    /// Reports counted so far. Never above `selected`: `record` admits a
    /// report only while entries remain.
    fn counted(&self) -> usize {
        self.unchanged + self.changed + self.skipped + self.failed
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    /// Admit the report at position `reported`: only the next one, only while
    /// entries remain, only in a survey not already poisoned. A rejected
    /// report poisons the survey.
    pub fn record(&mut self, reported: usize, disposition: UpdateDisposition) -> bool {
        let counted = self.counted();
        if self.invalid || reported != counted || counted == self.selected {
            self.invalid = true;
            return false;
        }
        match disposition {
            UpdateDisposition::Unchanged => self.unchanged += 1,
            UpdateDisposition::Changed => self.changed += 1,
            UpdateDisposition::Skipped => self.skipped += 1,
            UpdateDisposition::Failed => self.failed += 1,
        }
        true
    }

    /// A summary, only for a valid survey that counted every selected entry
    /// and whose caller holds exactly that many reports.
    pub fn finish(self, reported: usize) -> Option<UpdateSummary> {
        if self.invalid || reported != self.selected || self.counted() != self.selected {
            None
        } else {
            Some(UpdateSummary { counts: self })
        }
    }
}

/// A complete survey: every selected entry was reported exactly once.
pub struct UpdateSummary {
    counts: SurveyProgress,
}

impl UpdateSummary {
    pub fn selected(&self) -> usize {
        self.counts.selected
    }

    pub fn unchanged(&self) -> usize {
        self.counts.unchanged
    }

    pub fn changed(&self) -> usize {
        self.counts.changed
    }

    pub fn skipped(&self) -> usize {
        self.counts.skipped
    }

    pub fn failed(&self) -> usize {
        self.counts.failed
    }

    /// A failure outranks a change: a survey with any failed entry is
    /// incomplete whatever else changed.
    pub fn outcome(&self) -> UpdateCheckOutcome {
        if self.counts.failed != 0 {
            UpdateCheckOutcome::Incomplete
        } else if self.counts.changed != 0 {
            UpdateCheckOutcome::ChangesAvailable
        } else {
            UpdateCheckOutcome::Current
        }
    }

    /// The lock is written only in publish mode, with no failed entry and at
    /// least one change.
    pub fn publishes_lock(&self, mode: UpdateMode) -> bool {
        match mode {
            UpdateMode::Publish => self.counts.failed == 0 && self.counts.changed != 0,
            UpdateMode::Check => false,
        }
    }
}
