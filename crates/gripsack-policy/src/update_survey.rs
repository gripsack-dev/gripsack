//! Complete-only update accounting and the shared lock publication decision.
use vstd::prelude::*;

verus! {

#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub enum UpdateMode { Publish, Check }
#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub enum UpdateDisposition { Unchanged, Changed, Skipped, Failed }
#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub enum UpdateCheckOutcome { Current, ChangesAvailable, Incomplete }

#[derive(Debug)]
pub struct SurveyProgress {
    selected: usize,
    unchanged: usize,
    changed: usize,
    skipped: usize,
    failed: usize,
    invalid: bool,
}

impl SurveyProgress {
    #[verifier::type_invariant]
    spec fn bounded(&self) -> bool { self.counted() <= self.selected }

    pub closed spec fn selected(&self) -> usize { self.selected }
    pub closed spec fn unchanged(&self) -> usize { self.unchanged }
    pub closed spec fn changed(&self) -> usize { self.changed }
    pub closed spec fn skipped(&self) -> usize { self.skipped }
    pub closed spec fn failed(&self) -> usize { self.failed }
    pub closed spec fn invalid(&self) -> bool { self.invalid }
    pub closed spec fn counted(&self) -> nat {
        (self.unchanged as nat) + (self.changed as nat) + (self.skipped as nat) + (self.failed as nat)
    }

    pub fn new(selected: usize) -> (survey: Self)
        ensures survey.selected() == selected, survey.counted() == 0,
            survey.unchanged() == 0, survey.changed() == 0,
            survey.skipped() == 0, survey.failed() == 0, !survey.invalid(),
    { Self { selected, unchanged: 0, changed: 0, skipped: 0, failed: 0, invalid: false } }

    fn counted_reports(&self) -> (count: usize)
        ensures count == self.counted(), count <= self.selected(),
        no_unwind
    {
        proof { use_type_invariant(self); }
        self.unchanged + self.changed + self.skipped + self.failed
    }

    pub fn selected_count(&self) -> (count: usize) ensures count == self.selected(), no_unwind { self.selected }

    pub fn record(&mut self, reported: usize, disposition: UpdateDisposition) -> (admitted: bool)
        ensures
            final(self).selected() == old(self).selected(),
            admitted <==> !old(self).invalid() && reported == old(self).counted()
                && old(self).counted() < old(self).selected(),
            final(self).invalid() <==> !admitted,
            final(self).counted() == old(self).counted() + if admitted { 1nat } else { 0nat },
            final(self).unchanged() == old(self).unchanged() + if admitted && disposition == UpdateDisposition::Unchanged { 1int } else { 0int },
            final(self).changed() == old(self).changed() + if admitted && disposition == UpdateDisposition::Changed { 1int } else { 0int },
            final(self).skipped() == old(self).skipped() + if admitted && disposition == UpdateDisposition::Skipped { 1int } else { 0int },
            final(self).failed() == old(self).failed() + if admitted && disposition == UpdateDisposition::Failed { 1int } else { 0int },
        no_unwind
    {
        proof { use_type_invariant(&*self); }
        let counted = self.counted_reports();
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

    pub fn finish(self, reported: usize) -> (summary: Option<UpdateSummary>)
        ensures
            summary.is_some() <==> !self.invalid() && self.counted() == self.selected()
                && reported == self.selected(),
            summary.is_some() ==> summary.unwrap().selected_count_spec() == self.selected()
                && summary.unwrap().unchanged_count() == self.unchanged()
                && summary.unwrap().changed_count() == self.changed()
                && summary.unwrap().skipped_count() == self.skipped()
                && summary.unwrap().failed_count() == self.failed(),
        no_unwind
    {
        if self.invalid || reported != self.selected || self.counted_reports() != self.selected { None }
        else { Some(UpdateSummary { counts: self }) }
    }
}

#[derive(Debug)]
pub struct UpdateSummary { counts: SurveyProgress }
impl UpdateSummary {
    #[verifier::type_invariant]
    spec fn complete(&self) -> bool { !self.counts.invalid() && self.counts.counted() == self.counts.selected() }

    pub closed spec fn selected_count_spec(&self) -> usize { self.counts.selected() }
    pub closed spec fn unchanged_count(&self) -> usize { self.counts.unchanged() }
    pub closed spec fn changed_count(&self) -> usize { self.counts.changed() }
    pub closed spec fn skipped_count(&self) -> usize { self.counts.skipped() }
    pub closed spec fn failed_count(&self) -> usize { self.counts.failed() }

    pub fn selected(&self) -> (count: usize) ensures count == self.selected_count_spec(), no_unwind { self.counts.selected }
    pub fn unchanged(&self) -> (count: usize) ensures count == self.unchanged_count(), no_unwind { self.counts.unchanged }
    pub fn changed(&self) -> (count: usize) ensures count == self.changed_count(), no_unwind { self.counts.changed }
    pub fn skipped(&self) -> (count: usize) ensures count == self.skipped_count(), no_unwind { self.counts.skipped }
    pub fn failed(&self) -> (count: usize) ensures count == self.failed_count(), no_unwind { self.counts.failed }

    pub fn outcome(&self) -> (outcome: UpdateCheckOutcome)
        ensures outcome == if self.failed_count() != 0 { UpdateCheckOutcome::Incomplete }
            else if self.changed_count() != 0 { UpdateCheckOutcome::ChangesAvailable }
            else { UpdateCheckOutcome::Current },
        no_unwind
    {
        if self.counts.failed != 0 { UpdateCheckOutcome::Incomplete }
        else if self.counts.changed != 0 { UpdateCheckOutcome::ChangesAvailable }
        else { UpdateCheckOutcome::Current }
    }

    pub fn publishes_lock(&self, mode: UpdateMode) -> (publish: bool)
        ensures publish <==> mode == UpdateMode::Publish && self.failed_count() == 0 && self.changed_count() != 0,
        no_unwind
    { mode == UpdateMode::Publish && self.counts.failed == 0 && self.counts.changed != 0 }
}

}
