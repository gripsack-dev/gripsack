//! Scheduler transitions as a verified kernel (0047, handoff §5.6):
//! the ready-queue DECISIONS of `gripsack-exec::schedule::run_all`,
//! index-based and pure. The threaded bridge (mutex/condvar/worker
//! pool) stays in exec and stays tested, never wrapped and claimed
//! verified.
//!
//! Proved, over every interleaving of the transition functions:
//!
//! - **ready ⟹ predecessors completed successfully** — `next`
//!   returns a module only when every dependency finished OK (the
//!   `remaining` count is exactly the number of unfinished
//!   dependencies, by `unfinished_count`'s inductive linkage);
//! - **at most one start per module**;
//! - **a failure latches** — after `finish_fail` (or a protocol
//!   violation), `next` is always None;
//! - **finished ⟹ started**.
//!
//! Entry points are TOTAL: a `finish` for a module that never
//! started, finished twice, or arrives after the latch is a
//! worker-protocol violation and latches `failed`.
//!
//! Invariant architecture (Verus idioms, learned the hard way):
//! predicates are open spec METHODS reading fields directly (a
//! view-bundle indirection breaks the solver's congruence between
//! pre- and post-state, and Ghost struct fields poison spec field
//! access); a `&&&` chain must be the first token of a spec-fn body.
//! The deprecated postcondition style (`self` in ensures =
//! post-state) is deliberate: the new-mut-ref `final()` form
//! mis-evaluates method calls on the post-state for this shape.
//! Revisit at the next Verus pin bump.

use vstd::prelude::*;
// plain-cargo shim builds see no use of the seq/set lemmas; the
// `broadcast use` below (erased outside verification) needs them
#[allow(unused_imports)]
use vstd::{seq_lib::*, set_lib::*};

verus! {

broadcast use {
    group_seq_properties,
    group_set_properties,
    vstd::seq::Seq::lemma_take_succ_push,
    vstd::seq::lemma_seq_update_same,
    vstd::seq::lemma_seq_update_different,
    vstd::std_specs::vec::axiom_spec_len,
};

pub open spec fn unfinished_count(deps: Seq<usize>, finished: Seq<bool>) -> int
    decreases deps.len()
{
    if deps.len() == 0 {
        0
    } else {
        let last = deps[deps.len() - 1];
        if 0 <= last < finished.len() && finished[last as int] {
            unfinished_count(deps.drop_last(), finished)
        } else {
            unfinished_count(deps.drop_last(), finished) + 1
        }
    }
}

pub proof fn lemma_unfinished_count_zero(deps: Seq<usize>, finished: Seq<bool>)
    requires
        forall|i: int| #![trigger deps[i]] 0 <= i < deps.len() ==> 0 <= deps[i] < finished.len(),
    ensures
        unfinished_count(deps, finished) >= 0,
        // forward: zero count means everything finished
        unfinished_count(deps, finished) == 0 ==>
            (forall|i: int| #![trigger deps[i]] 0 <= i < deps.len() ==> finished[deps[i] as int]),
    decreases deps.len(),
{
    reveal(unfinished_count);
    if deps.len() > 0 {
        lemma_unfinished_count_zero(deps.drop_last(), finished);
        let dl = deps.drop_last();
        let last = deps[deps.len() - 1];
        // drop_last is pointwise the prefix
        assert forall|i: int| #![trigger dl[i]] 0 <= i < dl.len() implies dl[i] == deps[i] by {};
        assert(unfinished_count(deps, finished) ==
            (if 0 <= last < finished.len() && finished[last as int] {
                0int
            } else {
                1int
            }) + unfinished_count(deps.drop_last(), finished));
        // the goal, case-split for the solver
        assert(unfinished_count(deps, finished) == 0 ==>
            (forall|i: int| #![trigger deps[i]] 0 <= i < deps.len() ==> finished[deps[i] as int]))
        by {
            if unfinished_count(deps, finished) == 0 {
                assert(finished[last as int]);
                assert forall|i: int| #![trigger deps[i]] 0 <= i < deps.len() implies
                    finished[deps[i] as int]
                by {
                    if i == deps.len() - 1 {
                        assert(deps[i] == last);
                    } else {
                        assert(dl[i] == deps[i]);
                    }
                };
            }
        };
    }
}

/// The other direction: everything finished means zero count.
pub proof fn lemma_unfinished_count_zero_reverse(deps: Seq<usize>, finished: Seq<bool>)
    requires
        forall|i: int| #![trigger deps[i]] 0 <= i < deps.len() ==> 0 <= deps[i] < finished.len(),
        forall|i: int| #![trigger deps[i]] 0 <= i < deps.len() ==> finished[deps[i] as int],
    ensures
        unfinished_count(deps, finished) == 0,
    decreases deps.len(),
{
    reveal(unfinished_count);
    if deps.len() > 0 {
        lemma_unfinished_count_zero_reverse(deps.drop_last(), finished);
        let last = deps[deps.len() - 1];
        assert(unfinished_count(deps, finished) ==
            (if 0 <= last < finished.len() && finished[last as int] {
                0int
            } else {
                1int
            }) + unfinished_count(deps.drop_last(), finished));
    }
}

/// Marking `j` finished drops the count by its occurrence count —
/// at most one, by admission.
pub proof fn lemma_unfinished_count_decrement(
    deps: Seq<usize>,
    finished: Seq<bool>,
    j: usize,
)
    requires
        0 <= j < finished.len(),
        !finished[j as int],
        forall|i: int| #![trigger deps[i]] 0 <= i < deps.len() ==> 0 <= deps[i] < finished.len(),
        // j occurs at most once
        deps.no_duplicates(),
    ensures
        unfinished_count(deps, finished.update(j as int, true))
            == if deps.contains(j) {
                unfinished_count(deps, finished) - 1
            } else {
                unfinished_count(deps, finished)
            },
    decreases deps.len(),
{
    reveal(unfinished_count);
    if deps.len() > 0 {
        let dl = deps.drop_last();
        lemma_unfinished_count_decrement(dl, finished, j);
        lemma_unfinished_count_zero(dl, finished);
        lemma_unfinished_count_zero(dl, finished.update(j as int, true));
    }
}

/// Nothing finished ⟹ the count is the list's length.
pub proof fn lemma_unfinished_count_none_finished(deps: Seq<usize>, finished: Seq<bool>)
    requires
        forall|i: int| #![trigger finished[i]] 0 <= i < finished.len() ==> !finished[i],
        forall|i: int| #![trigger deps[i]] 0 <= i < deps.len() ==> 0 <= deps[i] < finished.len(),
    ensures
        unfinished_count(deps, finished) == deps.len(),
    decreases deps.len(),
{
    reveal(unfinished_count);
    if deps.len() > 0 {
        lemma_unfinished_count_none_finished(deps.drop_last(), finished);
        let last = deps[deps.len() - 1];
        assert(unfinished_count(deps, finished) ==
            (if 0 <= last < finished.len() && finished[last as int] {
                0int
            } else {
                1int
            }) + unfinished_count(deps.drop_last(), finished));
    }
}


/// A dependency list has no duplicate edges ⟹ its first k entries
/// exclude the k'th (the reversal's freshness driver).
pub proof fn lemma_fresh_entry(deps: &[Vec<usize>], from: int, k: int, d: usize)
    requires
        0 <= from < deps@.len(),
        0 <= k < deps@[from]@.len(),
        no_dup_edges(deps@),
        deps@[from]@[k] == d,
    ensures
        !deps@[from]@.take(k).contains(d),
{
    let full = deps@[from]@;
    if full.take(k).contains(d) {
        let j = choose|j: int|
            0 <= j < full.take(k).len() && full.take(k)[j] == d;
        assert(full.take(k)[j] == full[j]);
        assert(full[j] == d);
        // uniqueness: full[j] == d == full[k] with j < k contradicts
        // the admission
        assert(deps@[from]@[k] == d);
    }
}

/// Edge targets stay in range (the adapter builds the lists from the
/// name table) — a named predicate so callers, lemmas and invariants
/// share one symbol (quantified facts don't transfer across triggers).
pub open spec fn edges_in_range(deps: Seq<Vec<usize>>) -> bool {
    forall|i: int, idx: int| #![trigger deps[i]@[idx]]
        0 <= i < deps.len() && 0 <= idx < deps[i]@.len()
            ==> (deps[i]@[idx] as int) < deps.len()
}

/// No duplicate edges (a duplicated edge would double-decrement and
/// release a consumer early).
pub open spec fn no_dup_edges(deps: Seq<Vec<usize>>) -> bool {
    forall|i: int, a: int, b: int|
        #![trigger deps[i]@[a], deps[i]@[b]]
        0 <= i < deps.len()
        && 0 <= a < deps[i]@.len() && 0 <= b < deps[i]@.len()
        && deps[i]@[a] == deps[i]@[b] ==> a == b
}

/// The pure transition table behind the threaded scheduler.
/// All fields are public so the invariants can be open predicates —
/// the constructor stays the only maker.
pub struct PureScheduler {
    /// dependents[i]: modules waiting on i.
    pub dependents: Vec<Vec<usize>>,
    /// deps[i]: modules i waits on (the admission input).
    pub deps: Vec<Vec<usize>>,
    /// unfinished dependency count — the exec form readiness tests.
    pub remaining: Vec<usize>,
    pub started: Vec<bool>,
    pub finished: Vec<bool>,
    pub failed: bool,
    /// FIFO ready queue with a cursor (no VecDeque in specs).
    pub ready: Vec<usize>,
    pub head: usize,
    /// The ready WINDOW (ready[head..]) as a set — set membership has
    /// the solver's best lemmas. (Ghost WRAPPER, not a bare ghost
    /// field: bare ghost fields poison spec access to the struct.)
    pub queued: Ghost<Set<usize>>,
}

impl PureScheduler {
    /// Lengths and the queue cursor bound.
    pub open spec fn inv_shape(&self) -> bool {
        let n = self.deps@.len();
        &&& self.dependents@.len() == n
        &&& self.remaining@.len() == n
        &&& self.started@.len() == n
        &&& self.finished@.len() == n
        &&& self.head <= self.ready@.len()
    }

    /// Admission carried forward: every edge target is a valid index.
    pub open spec fn inv_in_range(&self) -> bool {
        let n = self.deps@.len();
        &&& edges_in_range(self.deps@)
        &&& no_dup_edges(self.deps@)
        &&& (forall|i: int, idx: int| #![trigger self.dependents@[i]@[idx]]
            0 <= i < n && 0 <= idx < self.dependents@[i]@.len()
                ==> (self.dependents@[i]@[idx] as int) < n)
        &&& (forall|k: int| 0 <= k < self.ready@.len() ==>
            (#[trigger] self.ready@[k]) < n)
    }

    /// deps and dependents are the same relation read both ways, and
    /// every dependents list is duplicate-free (a doubled edge would
    /// double-decrement and release a consumer early).
    pub open spec fn inv_link(&self) -> bool {
        let n = self.deps@.len();
        &&& (forall|i: usize, d: usize|
            #![trigger self.dependents@[d as int]@.contains(i)]
            #![trigger self.deps@[i as int]@.contains(d)]
            (i as int) < n && (d as int) < n ==>
                (self.deps@[i as int]@.contains(d)
                    <==> self.dependents@[d as int]@.contains(i)))
        &&& (forall|d: int| #![trigger self.dependents@[d]] 0 <= d < n ==>
            self.dependents@[d]@.no_duplicates())
    }

    /// `remaining` is exactly the unfinished-dependency count.
    pub open spec fn inv_remaining_exact(&self) -> bool {
        let n = self.deps@.len();
        forall|i: int| 0 <= i < n ==>
            self.remaining@[i] as int == unfinished_count(self.deps@[i]@, self.finished@)
    }

    /// Readiness is exact: queue members are unstarted and fully
    /// unblocked; nothing fully unblocked sits unqueued unstarted
    /// (while no failure is latched).
    pub open spec fn inv_readiness(&self) -> bool {
        let n = self.deps@.len();
        &&& (forall|k: int| #![trigger self.ready@[k]]
            self.head <= k < self.ready@.len() ==>
            !self.started@[self.ready@[k] as int]
            && self.remaining@[self.ready@[k] as int] == 0)
        &&& self.ready@.subrange(self.head as int, self.ready@.len() as int).to_set()
            =~= self.queued@
        &&& (forall|i: int| #![trigger self.started@[i], self.remaining@[i]]
            0 <= i < n && !self.started@[i]
                && self.remaining@[i] == 0 && !self.failed
                ==> self.queued@.contains(i as usize))
        // the window is duplicate-free
        &&& (forall|k1: int, k2: int|
            #![trigger self.ready@[k1], self.ready@[k2]]
            self.head <= k1 < self.ready@.len() && self.head <= k2 < self.ready@.len()
            && self.ready@[k1] == self.ready@[k2] ==> k1 == k2)
    }

    /// Safety and latching: a started module's dependencies are all
    /// finished; finished means started.
    pub open spec fn inv_safety(&self) -> bool {
        let n = self.deps@.len();
        &&& (forall|i: int| 0 <= i < n && self.started@[i] ==>
            forall|d: usize| #[trigger] self.deps@[i]@.contains(d)
                ==> self.finished@[d as int])
        &&& (forall|i: int| 0 <= i < n && self.finished@[i] ==> self.started@[i])
    }

    /// The whole-machine invariant.
    pub open spec fn inv(&self) -> bool {
        &&& self.inv_shape()
        &&& self.inv_in_range()
        &&& self.inv_link()
        &&& self.inv_remaining_exact()
        &&& self.inv_readiness()
        &&& self.inv_safety()
    }

    /// Build the transition table from per-module dependency lists.
    /// Admission: indices in range, no duplicate edges (a duplicated
    /// edge would double-decrement and release a consumer early).
    #[verifier::deprecated_postcondition_mut_ref_style(true)]
    pub fn new(deps: &[Vec<usize>]) -> (result: PureScheduler)
        requires
            edges_in_range(deps@),
            no_dup_edges(deps@),
        ensures
            result.inv(),
            result.ready@.len() > 0 ==> !result.failed,
            // initially ready: exactly the dependency-free modules
            forall|i: usize| #![trigger result.ready@.contains(i)]
                (i as int) < deps@.len() && deps@[i as int]@.len() == 0
                <==> result.ready@.contains(i),
    {
        let n = deps.len();
        // dependents, by edge reversal
        let mut dependents: Vec<Vec<usize>> = Vec::new();
        let mut i = 0;
        while i < n
            invariant
                i <= n == deps@.len(),
                dependents@.len() == i,
                forall|j: int| 0 <= j < i ==> dependents@[j]@.len() == 0,
            decreases n - i,
        {
            dependents.push(Vec::new());
            i += 1;
        }
        proof {
            assert forall|d: int, i: int| #![trigger dependents@[d]@[i]]
                0 <= d < n && 0 <= i < n implies
                !dependents@[d]@.contains(i as usize)
            by {
                assert(dependents@[d]@.len() == 0);
            };
        }
        let mut from = 0;
        while from < n
            invariant
                from <= n == deps@.len(),
                dependents@.len() == n,
                edges_in_range(deps@),
                no_dup_edges(deps@),
                forall|d: int, idx: int| #![trigger dependents@[d]@[idx]]
                    0 <= d < n && 0 <= idx < dependents@[d]@.len()
                        ==> (dependents@[d]@[idx] as int) < n,
                // reversal, exactly: dependents[d] holds the modules
                // before `from` listing d
                forall|d: usize, i: usize|
                    #![trigger dependents@[d as int]@.contains(i)]
                    #![trigger deps@[i as int]@.contains(d)]
                    (d as int) < n && (i as int) < n ==>
                    (dependents@[d as int]@.contains(i)
                        <==> ((i as int) < from && deps@[i as int]@.contains(d))),
                // entries are distinct (the pushed `from` is always
                // fresh — the cursor strictly increases)
                forall|d: int| #![trigger dependents@[d]] 0 <= d < n ==>
                    dependents@[d]@.no_duplicates(),
                forall|d: int, a: int| #![trigger dependents@[d]@[a]]
                    0 <= d < n && 0 <= a < dependents@[d]@.len()
                        ==> (dependents@[d]@[a] as int) < from,
            decreases n - from,
        {
            let mut k = 0;
            while k < deps[from].len()
                invariant
                    from < n == deps@.len(),
                    k <= deps@[from as int]@.len(),
                    dependents@.len() == n,
                    edges_in_range(deps@),
                    no_dup_edges(deps@),
                    forall|d: int, idx: int| #![trigger dependents@[d]@[idx]]
                        0 <= d < n && 0 <= idx < dependents@[d]@.len()
                            ==> (dependents@[d]@[idx] as int) < n,
                    forall|d: usize, i: usize|
                        #![trigger dependents@[d as int]@.contains(i)]
                        #![trigger deps@[i as int]@.contains(d)]
                        (d as int) < n && (i as int) < n ==>
                        (dependents@[d as int]@.contains(i)
                            <==> (((i as int) < from
                                || ((i as int) == from
                                    && deps@[i as int]@.take(k as int).contains(d)))
                                && deps@[i as int]@.contains(d))),
                    forall|d: int| #![trigger dependents@[d]] 0 <= d < n ==>
                        dependents@[d]@.no_duplicates(),
                    forall|d: int, a: int| #![trigger dependents@[d]@[a]]
                        0 <= d < n && 0 <= a < dependents@[d]@.len()
                            ==> (dependents@[d]@[a] as int) <= from,
                decreases deps@[from as int]@.len() - k,
            {
                let d = deps[from][k];
                proof {
                    // the exec read in spec form, up front
                    assert(deps@[from as int]@[k as int] == d);
                    // d is not among the first k edges of `from` (no
                    // duplicate edges)
                    lemma_fresh_entry(deps, from as int, k as int, d);
                    // so `from` is fresh for d's list: existing entries
                    // are strictly smaller (the head biconditional
                    // plus d's absence from the first k)
                    assert forall|a: int| #![trigger dependents@[d as int]@[a]]
                        0 <= a < dependents@[d as int]@.len() implies
                            (dependents@[d as int]@[a] as int) < from
                    by {
                        assert(dependents@[d as int]@.contains(dependents@[d as int]@[a])) by {
                            assert(0 <= a < dependents@[d as int]@.len());
                        };
                    };
                    assert(!dependents@[d as int]@.contains(from));
                }
                dependents[d].push(from);
                k += 1;
            }
            from += 1;
        }
        // initially ready: the dependency-free modules, in order
        let mut ready: Vec<usize> = Vec::new();
        let mut i = 0;
        while i < n
            invariant
                i <= n == deps@.len(),
                forall|j: usize| ready@.contains(j) <==>
                    (j as int) < i && deps@[j as int]@.len() == 0,
                forall|a: int, b: int| 0 <= a < ready@.len() && 0 <= b < ready@.len()
                    && ready@[a] == ready@[b] ==> a == b,
                forall|k: int| 0 <= k < ready@.len() ==> (#[trigger] ready@[k]) < n,
            decreases n - i,
        {
            if deps[i].is_empty() {
                proof {
                    assert(!ready@.contains(i as usize));
                }
                ready.push(i);
            }
            i += 1;
        }
        let mut remaining: Vec<usize> = Vec::new();
        let mut r = 0;
        while r < n
            invariant
                r <= n == deps@.len(),
                remaining@.len() == r,
                forall|i: int| 0 <= i < r ==> remaining@[i] == deps@[i]@.len(),
            decreases n - r,
        {
            remaining.push(deps[r].len());
            r += 1;
        }
        let mut deps_owned: Vec<Vec<usize>> = Vec::new();
        let mut c = 0;
        while c < n
            invariant
                c <= n == deps@.len(),
                deps_owned@.len() == c,
                forall|i: int| 0 <= i < c ==>
                    deps_owned@[i]@ == deps@[i]@,
            decreases n - c,
        {
            deps_owned.push(deps[c].clone());
            c += 1;
        }
        let sched = PureScheduler {
            dependents,
            deps: deps_owned,
            remaining,
            started: vec![false; n],
            finished: vec![false; n],
            failed: false,
            ready,
            head: 0,
            queued: Ghost(ready@.to_set()),
        };
        proof {
            // readiness of the initial queue: members are unblocked
            assert forall|k: int| #![trigger sched.ready@[k]] 0 <= k < sched.ready@.len()
                implies sched.remaining@[sched.ready@[k] as int] == 0
            by {
                assert(sched.ready@.contains(sched.ready@[k])) by {
                    assert(0 <= k < sched.ready@.len());
                };
                assert(sched.deps@[sched.ready@[k] as int]@.len() == 0);
            };
            // unblocked ⟹ queued
            assert forall|i: int| #![auto] 0 <= i < n && sched.remaining@[i] == 0 implies
                sched.queued@.contains(i as usize)
            by {
                assert(sched.deps@[i]@.len() == 0);
                assert(sched.ready@.contains(i as usize));
            };
            // the window IS the queue's set
            assert(sched.ready@.subrange(0, sched.ready@.len() as int).to_set()
                =~= sched.ready@.to_set());
            // remaining is the exact unfinished count: nothing is finished
            assert forall|i: int| #![auto] 0 <= i < n implies
                sched.remaining@[i] as int
                    == unfinished_count(sched.deps@[i]@, sched.finished@)
            by {
                lemma_unfinished_count_none_finished(sched.deps@[i]@, sched.finished@);
            };
        }
        sched
    }

    /// The next module to start, if any. None while a failure is
    /// latched (a failed dependency never authorizes a consumer) or
    /// the queue is drained.
    pub fn start_next(&mut self) -> (result: Option<usize>)
        requires
            old(self).inv(),
        ensures
            final(self).inv_shape(),
            final(self).inv_in_range(),
            final(self).inv_link(),
            final(self).inv_remaining_exact(),
            final(self).inv_readiness(),
            final(self).inv_safety(),
            final(self).inv(),
            result.is_some() ==> {
                let i = result.unwrap() as int;
                // THE safety property: every dependency finished OK
                &&& (forall|d: usize| #[trigger] final(self).deps@[i]@.contains(d)
                    ==> final(self).finished@[d as int])
                // at most one start: it was never started before
                &&& !old(self).started@[i]
                &&& final(self).started@[i]
            },
            old(self).failed ==> result.is_none(),
            old(self).failed ==> final(self).failed,
    {
        if self.failed || self.head >= self.ready.len() {
            return None;
        }
        // entry snapshots: post-mutation reasoning never touches
        // old(self) — local ghosts cannot be re-seated
        let ghost pre_started = self.started@;
        let ghost pre_remaining = self.remaining@;
        let ghost pre_queued = self.queued@;
        let ghost pre_ready = self.ready@;
        let ghost pre_head = self.head as int;
        let module = self.ready[self.head];
        proof {
            // queue-readiness: the popped module is fully unblocked,
            // so every dependency finished (the exact-count link)
            assert(!self.started@[module as int]);
            assert(self.remaining@[module as int] == 0);
            lemma_unfinished_count_zero(self.deps@[module as int]@, self.finished@);
        }
        self.head += 1;
        self.started[module] = true;
        proof {
            // the entry readiness clause, restated over the snapshots
            assert(forall|i: int| #![trigger pre_started[i], pre_remaining[i]]
                0 <= i < self.deps@.len() && !pre_started[i]
                    && pre_remaining[i] == 0 && !self.failed
                ==> pre_queued.contains(i as usize));
            // the popped module was queued
            assert(pre_queued.contains(module)) by {
                assert(pre_ready.subrange(pre_head, pre_ready.len() as int).contains(module)) by {
                    assert(pre_ready[pre_head] == module);
                };
            };
            self.queued = Ghost(pre_queued.remove(module));
            // the entry window was [module] plus the new one
            assert(pre_ready.subrange(pre_head, pre_ready.len() as int)
                == seq![module] + self.ready@.subrange(self.head as int, self.ready@.len() as int));
            assert forall|i: int|
                #![trigger pre_queued.contains(i as usize)]
                0 <= i < self.deps@.len() ==>
                (self.queued@.contains(i as usize)
                    <==> (pre_queued.contains(i as usize) && i as usize != module))
            by {
            };
            assert(self.ready@.subrange(self.head as int, self.ready@.len() as int).to_set()
                =~= self.queued@);
            assert forall|k: int| #![trigger self.ready@[k]]
                self.head <= k < self.ready@.len() implies
                !self.started@[self.ready@[k] as int]
                && self.remaining@[self.ready@[k] as int] == 0
            by {
                assert(pre_queued.contains(self.ready@[k]));
                assert(self.ready@[k] != module);
            };
            assert forall|i: int| #![trigger self.started@[i], self.remaining@[i]]
                0 <= i < self.deps@.len() && !self.started@[i]
                    && self.remaining@[i] == 0 && !self.failed
                implies self.queued@.contains(i as usize)
            by {
                assert(!pre_started[i]);
                assert(pre_remaining[i] == 0);
                assert(pre_queued.contains(i as usize));
                assert(self.deps@.len() <= usize::MAX as int) by {
                    assert(self.deps@.len() == vstd::std_specs::vec::spec_vec_len(&self.deps) as int);
                };
                lemma_cast_collapse(i, module);
                if i as usize == module {
                    assert(self.started@[module as int]); // the write
                    assert(false); // i == module, but module started
                }
                assert(self.queued@.contains(i as usize));
            };
        }
        Some(module)
    }

    /// A module completed successfully: its dependents come one
    /// dependency closer to ready, and newly unblocked ones queue.
    /// A finish for a module that never started (or finished twice)
    /// is a worker-protocol violation — latches failed.
    #[verifier::deprecated_postcondition_mut_ref_style(true)]
    pub fn finish_ok(&mut self, module: usize)
        requires
            self.inv(),
        ensures
            self.inv_shape(),
            self.inv_in_range(),
            self.inv_link(),
            self.inv_remaining_exact(),
            self.inv_readiness(),
            self.inv_safety(),
            self.inv(),
            old(self).failed ==> self.failed,
    {
        if module >= self.deps.len()
            || !self.started[module]
            || self.finished[module]
            || self.failed
        {
            // protocol violations and post-failure completions both
            // latch: nothing further starts
            self.failed = true;
            return;
        }
        // settle dependents FIRST (the count stays exact through the
        // frontier), then mark finished — the mark drops each
        // dependent's count by exactly its one occurrence of module
        let ghost mut settled: Set<usize> = Set::empty();
        let mut k = 0;
        while k < self.dependents[module].len()
            invariant
                module < self.deps@.len(),
                // module's mark lands after the loop: it stays
                // unfinished (and started) throughout
                !self.finished@[module as int],
                self.started@[module as int],
                !self.failed,
                k <= self.dependents@[module as int]@.len(),
                self.inv_shape(),
                self.inv_in_range(),
                self.inv_link(),
                self.inv_safety(),
                // the frontier: remaining is the unfinished count, less
                // one for dependents already settled this loop
                forall|i: int| 0 <= i < self.deps@.len() ==>
                    self.remaining@[i] as int
                        == unfinished_count(self.deps@[i]@, self.finished@)
                            - (if settled.contains(i as usize) {
                                1int
                            } else {
                                0int
                            }),
                // dependents are duplicate-free (a doubled edge would
                // double-decrement)
                self.dependents@[module as int]@.no_duplicates(),
                // the settled set IS the settled prefix of the
                // dependents list
                settled =~= self.dependents@[module as int]@.take(k as int).to_set(),
                self.ready@.subrange(self.head as int, self.ready@.len() as int).to_set()
                    =~= self.queued@,
                forall|j: int| #![trigger self.ready@[j]]
                    self.head <= j < self.ready@.len() ==>
                    !self.started@[self.ready@[j] as int]
                    && self.remaining@[self.ready@[j] as int] == 0,
                forall|i: int| 0 <= i < self.deps@.len() && !self.started@[i]
                    && self.remaining@[i] == 0
                    ==> self.ready@.subrange(self.head as int, self.ready@.len() as int)
                        .contains(i as usize),
                forall|k1: int, k2: int|
                    #![trigger self.ready@[k1], self.ready@[k2]]
                    self.head <= k1 < self.ready@.len() && self.head <= k2 < self.ready@.len()
                    && self.ready@[k1] == self.ready@[k2] ==> k1 == k2,
            decreases self.dependents@[module as int]@.len() - k,
        {
            let ghost settled_old = settled;
            let ghost rem_old = self.remaining@;
            let dependent = self.dependents[module][k];
            proof {
                // dependent lists module: the exec read, then the link
                assert(self.dependents@[module as int]@[k as int] == dependent);
                assert(self.dependents@[module as int]@.contains(dependent)) by {
                    assert(0 <= k && (k as int) < self.dependents@[module as int]@.len());
                };
                // the link, read the other way: module is a dependency
                assert(self.deps@[dependent as int]@.contains(module));
                // module is not finished (we mark it after the loop)
                assert(!self.finished@[module as int]);
                // so dependent's unfinished count is positive
                lemma_unfinished_count_zero(self.deps@[dependent as int]@, self.finished@);
                assert(unfinished_count(self.deps@[dependent as int]@, self.finished@) != 0);
                // not yet settled this loop: dependents are
                // duplicate-free and settled is the take(k) prefix
                assert(!settled.contains(dependent)) by {
                    let s = self.dependents@[module as int]@;
                    if s.take(k as int).to_set().contains(dependent) {
                        let j = choose|j: int| 0 <= j < k && s.take(k as int)[j] == dependent;
                        assert(s[j] == dependent);
                        assert(s[j] == s[k as int]);
                        assert(j == k as int); // no_duplicates
                        assert(false);
                    }
                };
                assert(self.remaining@[dependent as int] > 0);
            }
            self.remaining[dependent] -= 1;
            proof {
                // the write's spec, materialized at the site
                assert(self.remaining@[dependent as int] as int
                    == rem_old[dependent as int] as int - 1);
                settled = settled.insert(dependent);
                // the take grew by exactly this dependent, as sets
                assert(self.dependents@[module as int]@.take(k as int + 1).to_set()
                    =~= settled) by {
                    let s = self.dependents@[module as int]@;
                    assert(s.take(k as int + 1) =~= s.take(k as int).push(dependent));
                    assert(settled_old =~= s.take(k as int).to_set());
                };
            }
            if self.remaining[dependent] == 0 {
                proof {
                    // newly unblocked, and it never started (a started
                    // module has no unfinished dependencies — the count
                    // is exact)
                    assert(!self.started@[dependent as int]);
                }
                self.ready.push(dependent);
                proof {
                    self.queued = Ghost(self.queued@.insert(dependent));
                }
            }
            k += 1;
            proof {
                // the frontier advances by exactly this dependent
                assert forall|i: int| 0 <= i < self.deps@.len() implies
                    self.remaining@[i] as int
                        == unfinished_count(self.deps@[i]@, self.finished@)
                            - (if settled.contains(i as usize) {
                                1int
                            } else {
                                0int
                            })
                by {
                    assert(self.deps@.len() <= usize::MAX as int) by {
                        assert(self.deps@.len() == vstd::std_specs::vec::spec_vec_len(&self.deps) as int);
                    };
                    lemma_cast_collapse(i, dependent);
                    if i as usize == dependent {
                        assert(settled.contains(dependent));
                        // the head instance, materialized
                        assert(rem_old[i] as int == unfinished_count(
                            self.deps@[i]@,
                            self.finished@,
                        ));
                        // the write dropped exactly this entry by one
                        assert(self.remaining@ =~= rem_old.update(
                            dependent as int,
                            self.remaining@[dependent as int],
                        ));
                        assert(self.remaining@[i] as int + 1 == rem_old[i] as int);
                    } else {
                        assert(settled == settled_old.insert(dependent));
                        assert(rem_old[i] as int == unfinished_count(
                            self.deps@[i]@,
                            self.finished@,
                        ) - (if settled_old.contains(i as usize) {
                            1int
                        } else {
                            0int
                        }));
                        assert(self.remaining@[i] == rem_old[i]);
                    }
                };
            }
        }
        proof {
            // the mark drops each dependent's count by exactly its one
            // occurrence of module — and no other list's
            assert forall|i: int| 0 <= i < self.deps@.len() implies
                unfinished_count(self.deps@[i]@, self.finished@.update(module as int, true))
                    == if self.deps@[i]@.contains(module) {
                        unfinished_count(self.deps@[i]@, self.finished@) - 1
                    } else {
                        unfinished_count(self.deps@[i]@, self.finished@)
                    }
            by {
                // the graph-level admission gives this list's
                // uniqueness; the lemma covers both cases
                assert(self.deps@[i]@.no_duplicates());
                lemma_unfinished_count_decrement(self.deps@[i]@, self.finished@, module);
            };
            // every dependent settled: the settled set IS the
            // dependents list as a set
            assert(settled =~= self.dependents@[module as int]@.to_set());
        }
        self.finished[module] = true;
    }

    /// A module failed: latch. Dependents never see the decrement, so
    /// they never become ready; the latch means nothing new starts.
    #[verifier::deprecated_postcondition_mut_ref_style(true)]
    pub fn finish_fail(&mut self, module: usize)
        requires
            self.inv(),
        ensures
            self.inv(),
            self.failed,
    {
        if module >= self.deps.len()
            || !self.started[module]
            || self.finished[module]
        {
            self.failed = true;
            return;
        }
        self.failed = true;
    }

    /// The failure latch.
    pub fn failed(&self) -> (result: bool)
        ensures
            result == self.failed,
    {
        self.failed
    }
}


/// int→usize casts of in-range ints are the identity: the two
/// comparison forms coincide.
pub proof fn lemma_cast_collapse(i: int, m: usize)
    requires
        0 <= i < usize::MAX,
    ensures
        i as usize == m <==> i == m as int,
{
}

}
