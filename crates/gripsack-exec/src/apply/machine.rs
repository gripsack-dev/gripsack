//! The host driver of the kernel's apply machine (plan/0056 F2 step 7).
//!
//! The machine (`gripsack_kernel::apply`) decides every whole-object unit,
//! whether a release is still claimed by this run's deploys, whether the run
//! cuts a generation, and when it may flip. The host walks the workspace in
//! its own order and drives the machine one unit at a time: it hands over the
//! unit's facts and its one observation, executes only the command the
//! machine returns — through the shared planner, whose op must install
//! exactly the identity the machine commanded, and the journal's leaf
//! protocol, which re-checks the observation before acting — and answers with
//! what landed. Managed blocks (K5) are settled by the host's own code when
//! the machine delegates them. The directory domain is the host's
//! sub-protocol (step 9): every directory step runs only when the machine
//! commands it ([`Machine::directory_step`]), and every destination the
//! domain holds is declared to it ([`Machine::hold`]).
use super::dests::Dests;
use crate::ctx::ExecError;
use gripsack_kernel::GenerationId;
use gripsack_kernel::apply::{
    ApplyState, Command, CommitInputs, Event, Outcome, Refusal, Settled, UnitFacts, Write,
    apply_step,
};
use gripsack_kernel::directory::{Action, DirectoryFacts};
use gripsack_kernel::ownership::Refusal as UnitRefusal;
use gripsack_kernel::recover::{Failure, Intended, Live};
use gripsack_kernel::selection::SelectionIdentity;
use std::collections::BTreeMap;

/// What the machine commands for one unit.
#[derive(Debug)]
pub(crate) enum Verdict {
    /// Execute the planned op: it must install `install` with this authority.
    Write { install: Intended, write: Write },
    /// The unit settled without a write.
    Settled(Settled),
    /// Settle it in the host's own domain, then answer [`Machine::delegated`].
    Delegate,
}

/// Whether the run cuts a generation.
pub(crate) enum Commit {
    Vacuous,
    Publish(GenerationId),
}

pub(crate) struct Machine {
    state: Option<ApplyState>,
    pending: Command,
    pub(crate) dests: Dests,
    owners: BTreeMap<String, u32>,
}

impl Machine {
    pub(crate) fn start() -> Result<Self, Refusal> {
        let mut machine = Self {
            state: Some(ApplyState::new()),
            pending: Command::Abort(Refusal::Protocol),
            dests: Dests::default(),
            owners: BTreeMap::new(),
        };
        match machine.step(Event::Start) {
            Command::ReadUnit(_) => Ok(machine),
            other => Err(unexpected(other)),
        }
    }

    fn step(&mut self, event: Event) -> Command {
        let command = match self.state.take() {
            Some(state) => {
                let (next, command) = apply_step(state, event);
                self.state = Some(next);
                command
            }
            None => Command::Abort(Refusal::Protocol),
        };
        self.pending = command;
        command
    }

    /// A managed block's owner as the kernel's slot number.
    pub(crate) fn owner(&mut self, owner: &str) -> u32 {
        let next = u32::try_from(self.owners.len()).unwrap_or(u32::MAX);
        *self.owners.entry(owner.to_owned()).or_insert(next)
    }

    /// Hand the machine one unit with its observation (none for a unit the
    /// machine delegates) and return what it commands.
    pub(crate) fn unit(
        &mut self,
        facts: UnitFacts,
        live: Option<Live>,
    ) -> Result<Verdict, Refusal> {
        if !matches!(self.pending, Command::ReadUnit(_)) {
            return Err(Refusal::Protocol);
        }
        match self.step(Event::UnitRead(facts)) {
            Command::Observe { dest, .. } => {
                let live = live.unwrap_or(Live::Unsupported);
                match self.step(Event::Observed { dest, live }) {
                    Command::Mutate { install, write, .. } => Ok(Verdict::Write { install, write }),
                    Command::Report { settled, .. } => self.reported(settled),
                    other => Err(unexpected(other)),
                }
            }
            Command::Delegate { .. } => Ok(Verdict::Delegate),
            Command::Report { settled, .. } => self.reported(settled),
            other => Err(unexpected(other)),
        }
    }

    /// The machine reported a settled unit; the host's progress line follows
    /// from the op, so the report is answered at once.
    fn reported(&mut self, settled: Settled) -> Result<Verdict, Refusal> {
        match self.step(Event::Done) {
            Command::ReadUnit(_) => Ok(Verdict::Settled(settled)),
            other => Err(unexpected(other)),
        }
    }

    /// The commanded write landed this object.
    pub(crate) fn landed(&mut self, live: Live) -> Result<(), Refusal> {
        if !matches!(self.pending, Command::Mutate { .. }) {
            return Err(Refusal::Protocol);
        }
        match self.step(Event::Landed { live }) {
            Command::Report {
                settled: Settled::Applied,
                ..
            } => self.reported(Settled::Applied).map(drop),
            other => Err(unexpected(other)),
        }
    }

    /// The commanded write failed (the caller reports its own error).
    pub(crate) fn failed(&mut self, failure: Failure) {
        self.step(Event::Failed(failure));
    }

    /// A delegated unit cannot be settled: the machine aborts (the caller
    /// reports its own error).
    pub(crate) fn refuse_delegated(&mut self) {
        self.step(Event::Refused);
    }

    /// A delegated unit settled, changing the disk or not.
    pub(crate) fn delegated(&mut self, mutated: bool) -> Result<(), Refusal> {
        if !matches!(self.pending, Command::Delegate { .. }) {
            return Err(Refusal::Protocol);
        }
        match self.step(Event::Delegated { mutated }) {
            Command::Report { settled, .. } => self.reported(settled).map(drop),
            other => Err(unexpected(other)),
        }
    }

    /// One step of the directory sub-protocol for the receipt rooted at
    /// `root`: handed to the machine as an item, and run by `execute` (which
    /// answers whether it changed the disk) only when the machine commands it
    /// — in stage order, for a destination no deploy of this run claims. A
    /// step that fails is answered refused, which aborts the machine.
    pub(crate) fn directory_step(
        &mut self,
        root: &std::path::Path,
        action: Action,
        execute: impl FnOnce() -> Result<bool, ExecError>,
    ) -> Result<(), ExecError> {
        let refused = |refusal| refusal_error(refusal, "*", root.to_str());
        let dest = self.dests.intern(root)?;
        if !matches!(self.pending, Command::ReadUnit(_)) {
            return Err(refused(Refusal::Protocol));
        }
        match self.step(Event::DirectoryRead(DirectoryFacts { dest, action })) {
            Command::Directory { .. } => {}
            other => return Err(refused(unexpected(other))),
        }
        let mutated = match execute() {
            Ok(mutated) => mutated,
            Err(error) => {
                self.step(Event::Refused);
                return Err(error);
            }
        };
        match self.step(Event::Directed { mutated }) {
            Command::Report { settled, .. } => self.reported(settled).map(drop).map_err(refused),
            other => Err(refused(unexpected(other))),
        }
    }

    /// The directory domain holds `node` (an owned node of an active
    /// receipt) for the rest of the run: no whole-object unit may write it,
    /// and a release there is the domain's.
    pub(crate) fn hold(&mut self, node: &std::path::Path) -> Result<(), ExecError> {
        let refused = |refusal| refusal_error(refusal, "*", node.to_str());
        let dest = self.dests.intern(node)?;
        if !matches!(self.pending, Command::ReadUnit(_)) {
            return Err(refused(Refusal::Protocol));
        }
        let facts = DirectoryFacts {
            dest,
            action: Action::Hold,
        };
        match self.step(Event::DirectoryRead(facts)) {
            Command::Report {
                settled: Settled::Held,
                ..
            } => self.reported(Settled::Held).map(drop).map_err(refused),
            other => Err(refused(unexpected(other))),
        }
    }

    /// Every unit and directory item was handed over: the machine asks for
    /// the commit facts.
    pub(crate) fn exhausted(&mut self) -> Result<(), Refusal> {
        match self.step(Event::Exhausted) {
            Command::ReadCommit => Ok(()),
            other => Err(unexpected(other)),
        }
    }

    /// Whether and what the run publishes (K6, with the machine's own record
    /// of whether it changed the disk).
    pub(crate) fn commit(&mut self, inputs: CommitInputs) -> Result<Commit, Refusal> {
        match self.step(Event::CommitRead(inputs)) {
            Command::Finish(Outcome::Vacuous) => Ok(Commit::Vacuous),
            Command::Publish(generation) => Ok(Commit::Publish(generation)),
            other => Err(unexpected(other)),
        }
    }

    /// The generation and its pending activation record are durable: the
    /// machine names the selection to flip to.
    pub(crate) fn published(&mut self) -> Result<SelectionIdentity, Refusal> {
        match self.step(Event::Done) {
            Command::Flip(selection) => Ok(selection),
            other => Err(unexpected(other)),
        }
    }

    /// The flip is durable: the machine hands over to activation.
    pub(crate) fn flipped(&mut self) -> Result<(), Refusal> {
        match self.step(Event::Done) {
            Command::Activate => Ok(()),
            other => Err(unexpected(other)),
        }
    }

    /// Activation ran (whatever its hooks returned): the run is finished.
    pub(crate) fn activated(&mut self) -> Result<(), Refusal> {
        match self.step(Event::Done) {
            Command::Finish(Outcome::Applied) => Ok(()),
            other => Err(unexpected(other)),
        }
    }
}

fn unexpected(command: Command) -> Refusal {
    match command {
        Command::Abort(refusal) => refusal,
        _ => Refusal::Protocol,
    }
}

/// A refusal as the error the CLI shows. `to` is the declared spelling of
/// the unit the refusal names, when it names one.
pub(crate) fn refusal_error(refusal: Refusal, output: &str, to: Option<&str>) -> ExecError {
    let to = to.unwrap_or("a destination");
    let detail = match refusal {
        Refusal::Refused { why, .. } => match why {
            UnitRefusal::Unowned => format!(
                "{to} exists and was not deployed by gripsack — it is left untouched; move it away or use --take-over"
            ),
            UnitRefusal::Drifted => format!(
                "{to} changed since gripsack deployed it — it is left untouched; move it away or use --take-over"
            ),
            UnitRefusal::Unsupported => {
                format!("{to} is neither a regular file nor a symlink — move it aside to continue")
            }
        },
        Refusal::Drifted { .. } => format!(
            "{to} changed between the drift decision and the mutation — aborting; re-run to retry"
        ),
        Refusal::NotApplied { .. } => format!("{to} did not reach its intended state"),
        Refusal::Delegated { .. } => format!("{to} could not be settled"),
        Refusal::Order { .. } => {
            "a destination or directory step was handed over out of order (a gripsack bug)".into()
        }
        Refusal::Overlap { .. } => format!(
            "{to} is both a directory gripsack owns and a file it deploys — declare one or the other"
        ),
        Refusal::Blocked { .. } => format!(
            "{to} stands where gripsack must create a directory, and removing it would lose the file it replaced — move it away to continue"
        ),
        Refusal::Directory { .. } => format!("the directory transition at {to} failed"),
        Refusal::Unpublishable(why) => {
            format!("the new generation cannot be published ({why:?}) — refusing to activate it")
        }
        Refusal::Malformed => "the apply machine's counters overflowed".into(),
        Refusal::Failed => "an apply step failed".into(),
        Refusal::Protocol => "the apply machine and its driver disagree (a gripsack bug)".into(),
    };
    ExecError::Step {
        output: output.into(),
        step: "deploy".into(),
        detail,
    }
}

/// The op the planner built installs exactly what the machine commanded.
pub(crate) fn installs(op: &crate::ops::Op, install: &Intended) -> bool {
    gripsack_host::store::journal::kernel_intended(op.intended()).is_ok_and(|intended| {
        match (intended, install) {
            (Intended::Removed, Intended::Removed) => true,
            (Intended::Obj(planned), Intended::Obj(commanded)) => planned == *commanded,
            _ => false,
        }
    })
}

/// The op the planner built for a whole-object unit is the one the machine
/// commanded: the same write, authority and installed identity, or the same
/// inert outcome.
pub(crate) fn agrees(op: &crate::ops::Op, verdict: &Verdict) -> bool {
    use crate::ops::{Authority, OpKind};
    match verdict {
        Verdict::Write { install, write } => {
            matches!(
                op.kind(),
                OpKind::Link { .. } | OpKind::Write { .. } | OpKind::Remove(_)
            ) && installs(op, install)
                && matches!(
                    (op.authority(), write),
                    (Some(Authority::Fresh), Write::Create)
                        | (Some(Authority::Update), Write::Replace)
                        | (Some(Authority::TakeOver), Write::TakeOver)
                )
        }
        Verdict::Settled(Settled::Already) => matches!(op.kind(), OpKind::Satisfied),
        Verdict::Settled(Settled::Kept(_)) => matches!(op.kind(), OpKind::Preserved),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    //! The host lifting the deleted `ops/model.rs` explored, now checked
    //! against the kernel's own decision: for every case, the shipped planner
    //! builds exactly the op the apply machine's `decide_apply` commands.
    use super::*;
    use crate::DeploymentEntry;
    use crate::deploy::Observation;
    use crate::ops::{DestView, ModeInput, PreviousRecord, WritePermissions};
    use gripsack_host::store;
    use gripsack_host::store::Ownership;
    use gripsack_kernel::apply::{OnDrift, Slot, Step, Want, decide_apply};
    use gripsack_kernel::recover::Route;
    use std::path::Path;

    fn declared(mode: Ownership, to: &Path) -> DeploymentEntry {
        DeploymentEntry {
            from: "payload".into(),
            to: to.to_string_lossy().into_owned(),
            mode,
            vars: Default::default(),
            marker: None,
            span: None,
        }
    }

    fn recorded(
        mode: Ownership,
        to: &Path,
        hash: store::hash::ManifestHash,
        preserved: bool,
    ) -> store::DeployedEntry {
        store::DeployedEntry {
            from: "payload".into(),
            to: to.to_string_lossy().into_owned(),
            key: Some(to.to_path_buf()),
            ownership: store::StoredOwnership::Legacy(mode),
            vars: Default::default(),
            hash,
            file_mode: Some(0o644),
            source_executable: Some(false),
            prior: None,
            preserved_drift: preserved,
        }
    }

    fn verdict(step: Step) -> Result<Verdict, Refusal> {
        match step {
            Step::Write { install, write } => Ok(Verdict::Write { install, write }),
            Step::Leave(settled) => Ok(Verdict::Settled(settled)),
            Step::Refuse(refusal) => Err(refusal),
        }
    }

    #[test]
    fn the_planner_builds_exactly_the_op_the_kernel_commands() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("dest");
        let store_path = dir.path().join("store");
        let source = store_path.join("payload");
        let desired: &[u8] = b"desired\n";
        let file = |bytes: &[u8], mode: u32| Observation::File {
            bytes: bytes.to_vec(),
            mode,
        };
        let link = |target: &Path| Observation::Symlink {
            target: target.as_os_str().to_owned(),
        };
        let lives = || {
            [
                None,
                Some(file(desired, 0o644)),
                Some(file(b"previous\n", 0o644)),
                Some(file(b"user\n", 0o600)),
                Some(link(&source)),
                Some(link(Path::new("/elsewhere"))),
            ]
        };
        let mut cases = 0;
        for mode in [Ownership::TrackedCopy, Ownership::Owned] {
            let entry = declared(mode.clone(), &dest);
            // the previous generation: nothing, gripsack's write, preserved drift
            let own = match mode {
                Ownership::Owned => store::canonical_bytes_hash(b"x").into(),
                _ => store::canonical_bytes_identity(b"previous\n", 0o644).into(),
            };
            let records = [
                None,
                Some(recorded(mode.clone(), &dest, own, false)),
                Some(recorded(
                    mode.clone(),
                    &dest,
                    store::canonical_bytes_identity(b"user\n", 0o600).into(),
                    true,
                )),
            ];
            for index in 0..lives().len() {
                for record in &records {
                    for take_over in [false, true] {
                        cases += 1;
                        let view = DestView {
                            output: "m",
                            entry: &entry,
                            dest: dest.clone(),
                            home: dir.path(),
                            observed: lives().into_iter().nth(index).unwrap(),
                            prev: record.as_ref().map(|entry| PreviousRecord {
                                entry,
                                store_path: &store_path,
                            }),
                            consent: crate::ops::take_over_consent(take_over),
                        };
                        let input = match mode {
                            Ownership::Owned => ModeInput::Link {
                                source: &source,
                                content_hash: store::canonical_bytes_hash(desired).into(),
                            },
                            _ => ModeInput::Write {
                                content: desired,
                                permissions: WritePermissions::Source { executable: false },
                            },
                        };
                        let want = crate::ops::unit_want(&view, &input).unwrap();
                        let facts = UnitFacts {
                            dest: gripsack_kernel::DestId::new(0),
                            slot: Slot::Whole,
                            route: Route::Leaf,
                            recorded: want.recorded,
                            want: Want::Deploy {
                                ident: want.ident,
                                adopted: want.adopted,
                            },
                            consent: view.consent,
                            drift: match mode {
                                Ownership::Owned => OnDrift::Stop,
                                _ => OnDrift::Keep,
                            },
                        };
                        let live = crate::ops::kernel_live(view.observed.as_ref());
                        let label = format!(
                            "{mode:?} live={live:?} record={} take_over={take_over}",
                            record.as_ref().map_or("none", |r| if r.preserved_drift {
                                "preserved"
                            } else {
                                "own"
                            })
                        );
                        let op = crate::ops::plan_entry_op(&view, input).unwrap();
                        match verdict(decide_apply(0, &facts, &live)) {
                            Ok(verdict) => assert!(
                                agrees(&op, &verdict),
                                "{label}: {verdict:?} vs {:?}",
                                op.kind()
                            ),
                            // the planner renders a refused link as a kept
                            // object that needs --take-over; apply stops
                            Err(Refusal::Refused { .. }) => assert!(
                                matches!(op.kind(), crate::ops::OpKind::Preserved)
                                    && matches!(
                                        op.authority(),
                                        Some(
                                            crate::ops::Authority::Foreign
                                                | crate::ops::Authority::Drifted
                                        )
                                    ),
                                "{label}: refused but planned {:?}",
                                op.kind()
                            ),
                            Err(other) => panic!("{label}: {other:?}"),
                        }
                    }
                }
            }
        }
        assert_eq!(cases, 72);
    }
}
