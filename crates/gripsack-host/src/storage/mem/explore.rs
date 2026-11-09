//! Every crash cut × every crash outcome, bounded (plan/0056 §4, S3).
//!
//! `explore` runs a protocol once to count its effects, then once per cut
//! with a crash armed after that many effects, and hands every resulting
//! filesystem to a check: the kill image, and one power-loss image per
//! subset of the pending updates. The cut after the last effect is
//! included, so "returned before durability" is a power-loss violation
//! like any other.
use super::{MemFs, TraceEvent};
use std::fmt;
use std::io;

/// Which S3 crash kinds to enumerate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrashKinds {
    Kill,
    PowerLoss,
    Both,
}

impl CrashKinds {
    fn kill(self) -> bool {
        matches!(self, CrashKinds::Kill | CrashKinds::Both)
    }

    fn power_loss(self) -> bool {
        matches!(self, CrashKinds::PowerLoss | CrashKinds::Both)
    }
}

/// The most pending updates a power loss may choose among: the subsets are
/// enumerated exhaustively, so the bound keeps a test's cost explicit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingBound(pub usize);

impl Default for PendingBound {
    fn default() -> Self {
        Self(12)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Crash {
    Kill,
    /// Trace indices (absolute, see [`Violation::trace`]) of the effects
    /// that lost an update. An effect from before the exploration can be
    /// lost too, when `initial` itself had pending updates (after a kill).
    PowerLoss {
        lost: Vec<usize>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashPoint {
    /// Effects the protocol completed before the crash.
    pub cut: usize,
    /// The protocol had returned: the crash came after it.
    pub completed: bool,
    pub crash: Crash,
}

/// How much was explored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Exploration {
    pub cuts: usize,
    pub images: usize,
}

/// A failed check with the crash that produced it.
#[derive(Debug, Clone)]
pub struct Violation {
    pub point: CrashPoint,
    /// The whole trace of the crashed filesystem.
    pub trace: Vec<TraceEvent>,
    /// Trace index of the explored protocol's first effect.
    pub first: usize,
    pub failure: String,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "{} after {} effect(s){}: {:?}",
            self.failure,
            self.point.cut,
            if self.point.completed {
                " (protocol returned)"
            } else {
                ""
            },
            self.point.crash
        )?;
        let lost = match &self.point.crash {
            Crash::PowerLoss { lost } => lost.as_slice(),
            Crash::Kill => &[],
        };
        for (index, event) in self.trace.iter().enumerate() {
            let mark = if lost.contains(&index) {
                "LOST"
            } else if index == self.first {
                "--->"
            } else {
                "    "
            };
            writeln!(f, "  {mark} {index:>3} {event}")?;
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum ExploreError {
    /// The protocol failed without an injected crash, or behaved differently
    /// on a replay (protocols must be deterministic).
    Run(io::Error),
    /// More pending updates than the bound at this cut.
    Unbounded {
        cut: usize,
        pending: usize,
    },
    Violation(Box<Violation>),
}

impl fmt::Display for ExploreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExploreError::Run(error) => write!(f, "protocol run failed: {error}"),
            ExploreError::Unbounded { cut, pending } => {
                write!(f, "{pending} pending updates at cut {cut} exceed the bound")
            }
            ExploreError::Violation(violation) => violation.fmt(f),
        }
    }
}

impl std::error::Error for ExploreError {}

impl ExploreError {
    pub fn violation(&self) -> Option<&Violation> {
        match self {
            ExploreError::Violation(violation) => Some(violation),
            _ => None,
        }
    }
}

/// Run `protocol` from `initial`, crash it after every prefix of its
/// effects, and `check` every resulting filesystem. `initial` is never
/// modified. The first failed check is returned with its crash point.
pub fn explore(
    initial: &MemFs,
    kinds: CrashKinds,
    bound: PendingBound,
    protocol: impl Fn(&MemFs) -> io::Result<()>,
    mut check: impl FnMut(MemFs, &CrashPoint) -> Result<(), String>,
) -> Result<Exploration, ExploreError> {
    let base = initial.effects();
    let total = {
        let fs = initial.snapshot();
        protocol(&fs).map_err(ExploreError::Run)?;
        fs.effects() - base
    };
    let mut exploration = Exploration::default();
    for cut in 0..=total {
        let fs = initial.snapshot();
        fs.crash_at(base + cut);
        let completed = cut == total;
        match protocol(&fs) {
            Ok(()) if completed => {}
            Err(error) if !completed && MemFs::is_crash(&error) => {}
            Ok(()) => {
                return Err(ExploreError::Run(io::Error::other(format!(
                    "replay finished after {cut} of {total} effects"
                ))));
            }
            Err(error) => return Err(ExploreError::Run(error)),
        }
        exploration.cuts += 1;
        let mut verify = |image: MemFs, crash: Crash| {
            exploration.images += 1;
            let point = CrashPoint {
                cut,
                completed,
                crash,
            };
            let trace = image.trace();
            check(image, &point).map_err(|failure| {
                ExploreError::Violation(Box::new(Violation {
                    point,
                    trace,
                    first: base,
                    failure,
                }))
            })
        };
        if kinds.kill() {
            verify(fs.kill(), Crash::Kill)?;
        }
        if kinds.power_loss() {
            let pending = fs.pending_updates();
            if pending > bound.0 {
                return Err(ExploreError::Unbounded { cut, pending });
            }
            for subset in 0u64..(1u64 << pending) {
                let (image, lost) = fs.power_loss(|i| subset & (1 << i) != 0);
                verify(image, Crash::PowerLoss { lost })?;
            }
        }
    }
    Ok(exploration)
}
