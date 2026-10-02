#!/usr/bin/env python3
"""Production Verus gate with counted families and attributable semantic mutants."""
from dataclasses import dataclass
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

from verus_evidence import Evidence, EvidenceError, self_check

ROOT = Path(__file__).resolve().parent.parent
CRATE = ROOT / "crates/gripsack-policy"
MIN_OBLIGATIONS = 440
# Actual successful SMT function queries, excluding generated clone/spec-only
# helpers. One family cannot disappear behind growth in an unrelated module.
FAMILIES = {
    "recovery": tuple("selection::" + name for name in (
        "TransactionId::from_bytes", "TransactionId::as_bytes",
        "SelectionIdentity::legacy", "SelectionIdentity::transaction",
        "SelectionIdentity::generation", "SelectionIdentity::transaction_id", "classify",
    )),
    "activation": tuple("activation::" + name for name in (
        "AttemptNumber::admit", "AttemptNumber::first", "AttemptNumber::value",
        "AttemptNumber::checked_next", "IntentState::attempt",
        "next_attempt", "finish_attempt", "supersede",
    )),
    "ownership": ("ownership::plan_copy", "ownership::plan_link"),
    "retention": tuple("retention::" + name for name in (
        "admit_gc", "contains_generation", "contains_identity", "plan_prune", "plan_delete",
        "lemma_delete_monotone", "lemma_delete_monotone_prefix",
    )),
    "generation-inventory": tuple("generation::" + name for name in (
        "GenerationId::new", "GenerationId::value", "GenerationId::checked_next",
        "GenerationId::checked_previous", "GenerationInventory::new",
        "GenerationInventory::as_slice", "GenerationList::new", "GenerationList::inventory",
    )),
    "merge": tuple("merge::" + name for name in (
        "splice_bytes", "lemma_rest_is_gaps", "lemma_splice_canonical", "lemma_splice_identity",
        "lemma_gaps_end_with_tail", "lemma_splice_preserves_edges",
    )),
    "merge-scanner": tuple("merge::scanner::" + name for name in (
        "span_text", "lemma_ascii_successor", "next_line_end", "scan_admits_splice", "scan",
    )),
    "graph": ("graph::build_closure", "graph::build_only_members", "graph::lemma_reachable_visited"),
    "graph-roles": ("graph::roles::project_graph_roles", "graph::roles::project_graph_role", "graph::roles::GraphRole::is_dependency"),
    "graph-name-index": tuple("graph::name_index::" + name for name in (
        "bind_output_index", "BoundOutputIndex::position", "distinct_names_have_distinct_indices",
    )),
    "collector-induction": tuple("graph::collector::" + name for name in (
        "expand_union", "expand_insert", "expand_push", "expand_take",
    )),
    "target": ("target::release_at_most", "target::supports_target"),
    "conda-admission": tuple("conda_admission::" + name for name in (
        "terminal_succeeded", "admit_response",
    )),
    "workspace-command": tuple("workspace_command::" + name for name in (
        "admit_directory", "admit_argument", "admit_bash_options", "admit_production_binding",
        "interpreter_and_environment_roles_are_disjoint",
        "isolated_execution_never_admits_live_host_directory",
    )),
    "buildkit-definition": tuple("buildkit::" + name for name in (
        "backward_edge", "isolated_execution", "output_mount", "VertexIndex::new",
        "input_binding", "transition", "done_requires_export", "terminals_are_fenced",
    )),
    "owned-worker": tuple("worker_lease::" + name for name in (
        "bytes_equal", "identity_decision", "admit_acquire", "admit_stop",
        "admit_removal", "release_disposition", "admit_cache_cleanup", "retired_epoch",
        "admit_replacement",
    )),
    "scheduler": tuple("schedule::" + name for name in (
        "PureScheduler::new", "PureScheduler::start_next", "PureScheduler::finish_ok",
        "PureScheduler::finish_fail", "PureScheduler::failed", "lemma_unfinished_count_zero",
        "lemma_unfinished_count_zero_reverse", "lemma_unfinished_count_decrement",
        "lemma_unfinished_count_none_finished", "lemma_fresh_entry", "lemma_cast_collapse",
    )),
    "rate-admission": (
        "rate_limit::binary_rate::admit_binary_rate",
        "rate_limit::binary_rate::RatePeriod::nanoseconds",
        "rate_limit::binary_rate::BinaryTokenRate::components",
        "rate_limit::legacy_credits::admit_legacy_credits",
        "rate_limit::TokenBucket::from_legacy",
        "rate_limit::TokenBucket::restored",
        "rate_limit::TokenBucket::reconfigure",
    ),
    "token-transitions": (
        "rate_limit::capacity_for", "rate_limit::deficit_wait", "rate_limit::rate_bounds",
        "rate_limit::TokenBucket::full", "rate_limit::TokenBucket::refill",
        "rate_limit::TokenBucket::take",
    ),
    "credit-encoding": ("rate_limit::credit_balance::three_word_encoding",),
    "credit-arithmetic": tuple("rate_limit::credit_balance::CreditBalance::" + name for name in (
        "from_small", "from_shifted", "to_small", "compare", "clamp", "add_capped",
        "rescale_capped", "add_assign", "multiply_assign", "subtract_assign", "divide_assign",
    )) + tuple("rate_limit::credit_balance::credit_words::" + name for name in (
        "add_word", "subtract_word", "multiply_word", "divide_word",
    )),
    "operation-deadline": (
        "operation_budget::RemainingTime::nanoseconds",
        "operation_budget::OperationBudget::new",
        "operation_budget::OperationBudget::observe",
        "operation_budget::OperationBudget::admit_wait",
        "operation_budget::OperationBudget::stop",
    ),
    "http-attempt-budget": tuple("retry_budget::RetryBudget::" + name for name in (
        "new", "attempt_count", "waited_nanoseconds", "stop", "begin", "decide", "complete",
    )),
    "process-input": tuple("process_budget::input::" + name for name in (
        "InputByteLimit::new", "InputByteLimit::bytes",
        "InputAppend::reserve_additional", "admit_input_append",
    )),
    "process-input-transfer": tuple("process_budget::transfer::InputTransfer::" + name for name in (
        "admit", "begin", "complete", "retry", "close", "is_closed",
    )) + ("process_budget::transfer::InputChunk::range",),
    "process-framing": tuple("process_budget::frame::" + name for name in (
        "FrameByteLimit::new", "FrameByteLimit::bytes",
        "FrameBudget::new", "FrameBudget::observe", "FrameBudget::finish",
    )),
    "process-output-bytes": tuple("process_budget::output::" + name for name in (
        "StdoutByteLimit::new", "StdoutByteLimit::bytes",
        "StderrByteLimit::new", "StderrByteLimit::bytes",
        "RetainedStderrLimit::new", "RetainedStderrLimit::bytes",
        "ObservedBytes::zero", "ObservedBytes::record",
        "StdoutBudget::new", "StdoutBudget::observe",
        "StderrBudget::new", "StderrBudget::observe", "StderrBudget::truncated",
    )),
    "process-retained-tail": tuple("process_budget::tail::" + name for name in (
        "retain_tail", "TailAppend::discard_bytes", "TailAppend::skip_bytes",
    )),
    "process-lifecycle": tuple("process_budget::lifecycle::ChildLifecycle::" + name for name in (
        "new", "observe_action", "observe_exit", "lose_ownership",
        "signal_action", "signal_succeeded", "needs_termination",
        "should_observe_for_reap", "begin_reap", "observe_reap",
        "cleanup_decision", "finish", "is_finished",
    )),
    "update-survey": tuple("update_survey::SurveyProgress::" + name for name in (
        "new", "counted_reports", "selected_count", "record", "finish",
    )) + tuple("update_survey::UpdateSummary::" + name for name in (
        "selected", "unchanged", "changed", "skipped", "failed", "outcome", "publishes_lock",
    )),
    "journal-publication": tuple("journal_protocol::RecordPublication::" + name for name in (
        "new", "action", "acknowledge", "finish",
    )),
    "journal-mutation-authority": ("journal_protocol::admit_mutation",),
    "journal-cleanup": tuple("journal_protocol::CleanupProgress::" + name for name in (
        "new", "action", "acknowledge",
    )),
    "semantic-framing": tuple("semantic::" + name for name in (
        "length_header", "lemma_le64_injective", "lemma_encode_frames_injective",
    )),
    "semantic-order": tuple("semantic::" + name for name in (
        "bytes_cmp", "bytes_leq", "lemma_compare_bytes_transitive",
        "lemma_compare_bytes_antisymmetric", "lemma_compare_bytes_equal",
        "lemma_leq_bytes_total_ordering", "lemma_leq_pair_total_ordering",
    )),
    "semantic-canonical": tuple("semantic::" + name for name in (
        "canonical_entries", "lemma_sorted_insert", "lemma_nodup_insert",
        "lemma_dedup_sorted_properties", "lemma_canonical_form_properties",
        "lemma_canonical_form_unique", "lemma_sorted_nodup_unique",
        "lemma_canonical_entries_match",
    )),
    "semantic-relation": tuple("semantic::" + name for name in (
        "lemma_unordered_presentation_invariant", "lemma_unordered_content_injective",
        "lemma_ordered_sensitive", "lemma_fluent_object_equivalence",
        "lemma_environment_change_invalidates", "lemma_provenance_cannot_change_identity",
        "lemma_semantic_change_invalidates_identity",
    )),
    "graph-projection": (
        "graph::roles::lemma_role_decision_total",
        "graph::lemma_projected_build_edge_in_closure",
        "graph::lemma_projected_validation_edge_retained",
    ),
}


@dataclass(frozen=True)
class Mutant:
    name: str
    file: str
    function: str
    before: str
    after: str
    also_fails: tuple[str, ...] = ()


MUTANTS = (
    Mutant("conda-native-terminal", "conda_admission.rs", "conda_admission::terminal_succeeded",
           "    matches!(terminal, NativeTerminal::Exited(0))\n", "    matches!(terminal, NativeTerminal::Exited(_))\n"),
    Mutant("conda-terminal-bypass", "conda_admission.rs", "conda_admission::admit_response",
           "    if !terminal_succeeded(terminal) {", "    if false {"),
    Mutant("conda-live-attempt", "conda_admission.rs", "conda_admission::admit_response",
           "    if expected.attempt != actual.attempt {", "    if false {"),
    Mutant("conda-platform", "conda_admission.rs", "conda_admission::admit_response",
           "    if expected.platform != actual.platform {", "    if false {"),
    Mutant("conda-operation", "conda_admission.rs", "conda_admission::admit_response",
           "        _ => Err(AdmissionRefusal::Operation),", "        _ => Ok(()),"),
    Mutant("conda-full-closure", "conda_admission.rs", "conda_admission::admit_response",
           "            if ed != ad {", "            if false {"),
    Mutant("conda-final-prefix", "conda_admission.rs", "conda_admission::admit_response",
           "            } else if ep != ap {", "            } else if false {"),
    Mutant("buildkit-forward-producer", "buildkit.rs", "buildkit::backward_edge",
           "    producer < consumer\n", "    producer <= consumer\n"),
    Mutant("buildkit-network-isolation", "buildkit.rs", "buildkit::isolated_execution",
           "    network == NETWORK_NONE && security == SECURITY_SANDBOX\n", "    security == SECURITY_SANDBOX\n"),
    Mutant("buildkit-writable-input", "buildkit.rs", "buildkit::output_mount",
           "    kind == MOUNT_BIND && readonly == !selected\n", "    kind == MOUNT_BIND\n"),
    Mutant("buildkit-input-output-binding", "buildkit.rs", "buildkit::input_binding",
           "    actual.vertex.value == expected.vertex.value && actual.output == expected.output",
           "    actual.vertex.value == expected.vertex.value"),
    Mutant("buildkit-premature-done", "buildkit.rs", "buildkit::transition",
           "    ensures next == next_stage(stage, event),\n{\n    match (stage, event) {",
           "    ensures next == next_stage(stage, event),\n{\n    if event == SessionEvent::Done { return Some(SessionStage::Done); }\n    match (stage, event) {"),
    Mutant("worker-acquire-state", "worker_lease.rs", "worker_lease::admit_acquire",
           "    if phase != WorkerPhase::Ready {\n        Err(AcquireRefusal::NotReady)",
           "    if phase == WorkerPhase::Ready {\n        Err(AcquireRefusal::NotReady)"),
    Mutant("worker-live-stop", "worker_lease.rs", "worker_lease::admit_stop",
           "    } else if live > 0 {\n        Err(StopRefusal::LiveLeases)",
           "    } else if live > MAX_LIVE_LEASES {\n        Err(StopRefusal::LiveLeases)"),
    Mutant("worker-removal-identity", "worker_lease.rs", "worker_lease::admit_removal",
           "    if identity == IdentityDecision::Current {", "    if identity != IdentityDecision::Current {"),
    Mutant("worker-unconfirmed-release", "worker_lease.rs", "worker_lease::release_disposition",
           "    if cleanup_confirmed {", "    if true {"),
    Mutant("worker-foreign-cache", "worker_lease.rs", "worker_lease::admit_cache_cleanup",
           "    if !volume_owned {", "    if false {"),
    Mutant("worker-recovery-namespace", "worker_lease.rs", "worker_lease::retired_epoch",
           "    same_namespace && 0 < epoch && epoch < next_epoch\n", "    0 < epoch && epoch < next_epoch\n"),
    Mutant("worker-recovery-next-epoch", "worker_lease.rs", "worker_lease::retired_epoch",
           "    same_namespace && 0 < epoch && epoch < next_epoch\n", "    same_namespace && 0 < epoch && epoch <= next_epoch\n"),
    Mutant("worker-replacement-live-clients", "worker_lease.rs", "worker_lease::admit_replacement",
           "    } else if live > 0 {\n        Err(ReplacementRefusal::LiveLeases)",
           "    } else if live > MAX_LIVE_LEASES {\n        Err(ReplacementRefusal::LiveLeases)"),
    Mutant("worker-replacement-mid-transition", "worker_lease.rs", "worker_lease::admit_replacement",
           "    } else if phase != WorkerPhase::Ready && phase != WorkerPhase::Stopped {",
           "    } else if false {"),
    Mutant("classifier", "selection.rs", "selection::classify", "(Some(_), _) => Classification::Ambiguous,", "(Some(_), _) => Classification::Committed,"),
    Mutant("classifier-transaction-identity", "selection.rs", "selection::classify",
           "(Some(_), Some(current)) if current == facts.target =>",
           "(Some(_), Some(current)) if current.generation() == facts.target.generation() =>"),
    Mutant("activation-interrupted-attempt", "activation.rs", "activation::next_attempt",
           "match attempt.checked_next() {", "match Some(*attempt) {"),
    Mutant("activation-stale-outcome", "activation.rs", "activation::finish_attempt",
           "if *active == attempt =>", "if *active != attempt =>"),
    Mutant("activation-supersession", "activation.rs", "activation::supersede",
           "IntentState::Started { attempt } => IntentState::Superseded { last_attempt: Some(*attempt) },",
           "IntentState::Started { attempt } => IntentState::Started { attempt: *attempt },"),
    Mutant("ownership-drift", "ownership.rs", "ownership::plan_copy", "Some((written, false)) if live == written => CopyPlan::Update,", "Some((written, true)) if live == written => CopyPlan::Update,"),
    Mutant("gc-roots-in-deletion", "retention.rs", "retention::plan_delete", "if !contains_identity(referenced, c) {", "if contains_identity(referenced, c) {"),
    Mutant("gc-newest-prefix", "retention.rs", "retention::plan_prune",
           "let g = generations[i];", "let g = generations[generations.len() - 1 - i];"),
    Mutant("generation-duplicate", "generation.rs", "generation::GenerationInventory::new",
           "if ids[i - 1].number >= ids[i].number {", "if ids[i - 1].number > ids[i].number {"),
    Mutant("merge-splice", "merge.rs", "merge::splice_bytes", "    out.extend_from_slice(&text[cursor..]);", ""),
    Mutant("merge-scanner-utf8", "merge/scanner.rs", "merge::scanner::scan",
           "span: Span { start: active.start, end },",
           "span: Span { start: active.start + 1, end },"),
    Mutant("graph-closure", "graph.rs", "graph::build_closure", "                    result.push(target);", ""),
    Mutant("graph-validation", "graph/roles.rs", "graph::roles::project_graph_role", "RoleDecision { build: false, required_validation: true },", "RoleDecision { build: false, required_validation: false },"),
    Mutant("graph-name-index", "graph/name_index.rs", "graph::name_index::bind_output_index", "    let same_name = names[position] == declared;", "    let same_name = true;"),
    Mutant("target-abi", "target.rs", "target::supports_target", "Some(BinaryAbi::Gnu), Some(BinaryAbi::Gnu)", "Some(BinaryAbi::Gnu), Some(BinaryAbi::Musl)"),
    Mutant("workspace-host-directory", "workspace_command.rs", "workspace_command::admit_directory",
           "    !matches!((context, origin), (ExecutionContext::IsolatedLinux, DirectoryOrigin::LiveHost))",
           "    true"),
    Mutant("workspace-interpreter-role", "workspace_command.rs", "workspace_command::admit_argument",
           "ArgumentPosition::Interpreter => matches!(origin, ArgumentOrigin::PackageCommand),",
           "ArgumentPosition::Interpreter => true,"),
    Mutant("workspace-strict-bash", "workspace_command.rs", "workspace_command::admit_bash_options",
           'options.len() == 4 && options[0].as_str() == "-e" && options[1].as_str() == "-u"',
           'options.len() == 4 && options[0].as_str() == "-e" && options[1].as_str() == "-e"'),
    Mutant("workspace-staging-authority", "workspace_command.rs", "workspace_command::admit_production_binding",
           "    matches!(owner, CommandOwner::Production)",
           "    true"),
    Mutant("scheduler-latch", "schedule.rs", "schedule::PureScheduler::start_next", "        if self.failed || self.head >= self.ready.len() {", "        if self.head >= self.ready.len() {"),
    Mutant("rate-spend", "rate_limit.rs", "rate_limit::TokenBucket::take",
           "self.available.subtract_assign(&token);", ""),
    Mutant("rate-refill-unit", "rate_limit.rs", "rate_limit::TokenBucket::refill",
           "let earned = CreditBalance::from_shifted(amount, parts.1);",
           "let earned = CreditBalance::from_shifted(amount, 0);"),
    Mutant("rate-wait-ceiling", "rate_limit.rs", "rate_limit::deficit_wait",
           "let wait = quotient as u64 + 1;", "let wait = quotient as u64;"),
    Mutant("legacy-negative-credit", "rate_limit/legacy_credits.rs",
           "rate_limit::legacy_credits::admit_legacy_credits",
           "if bits >> 63 != 0 { return Some(CreditBalance::zero()); }",
           "if bits >> 63 == 0 { return Some(CreditBalance::zero()); }"),
    Mutant("credit-carry", "rate_limit/credit_balance/credit_words.rs",
           "rate_limit::credit_balance::credit_words::add_word",
           "let total = left as u128 + right as u128 + carry as u128;",
           "let total = left as u128 + right as u128;"),
    Mutant("credit-period-fraction", "rate_limit/credit_balance.rs",
           "rate_limit::credit_balance::CreditBalance::rescale_capped",
           "let fraction = fractional_product / (source_scale as u128);",
           "let fraction = 0u128;"),
    Mutant("operation-clock-rewind", "operation_budget.rs", "operation_budget::OperationBudget::observe",
           "} else if elapsed_ns > self.observed {", "} else if elapsed_ns < self.observed {"),
    Mutant("operation-wait-at-deadline", "operation_budget.rs", "operation_budget::OperationBudget::admit_wait",
           "if wait_ns < remaining.nanoseconds() { true }",
           "if wait_ns <= remaining.nanoseconds() { true }"),
    Mutant("operation-terminal-revival", "operation_budget.rs", "operation_budget::OperationBudget::stop",
           "        proof { use_type_invariant(&*self); }\n        self.terminal = true;",
           "        proof { use_type_invariant(&*self); }"),
    Mutant("http-overlapping-attempt", "retry_budget.rs", "retry_budget::RetryBudget::begin",
           "if self.phase != Phase::Ready { return AttemptAdmission::Stop(self.stop(RetryRefusal::ProtocolOrder)); }", ""),
    Mutant("http-minimum-wait", "retry_budget.rs", "retry_budget::RetryBudget::begin",
           "if observed < self.ready_after {", "if false && observed < self.ready_after {"),
    Mutant("http-error-precedence", "retry_budget.rs", "retry_budget::RetryBudget::decide",
           "if !retryable { return RetryDecision::Stop(self.stop(RetryRefusal::NonRetryable)); }",
           "if !retryable && elapsed_ns < OPERATION_NANOSECONDS { return RetryDecision::Stop(self.stop(RetryRefusal::NonRetryable)); }"),
    Mutant("http-backoff", "retry_budget.rs", "retry_budget::RetryBudget::decide",
           "let backoff = if self.attempts == 1 { FIRST_RETRY_NANOSECONDS } else { 2 * FIRST_RETRY_NANOSECONDS };",
           "let backoff = FIRST_RETRY_NANOSECONDS;"),
    Mutant("http-cumulative-wait", "retry_budget.rs", "retry_budget::RetryBudget::decide",
           "self.waited_ns += delay as u64;", ""),
    Mutant("http-late-completion", "retry_budget.rs", "retry_budget::RetryBudget::complete",
           "if self.time.observe(elapsed_ns).is_none() { return Err(self.stop(RetryRefusal::Deadline)); }", ""),
    Mutant("process-input-boundary", "process_budget/input.rs", "process_budget::input::admit_input_append",
           "if additional > limit.bytes - length {", "if additional >= limit.bytes - length {"),
    Mutant("process-frame-delimiter", "process_budget/frame.rs", "process_budget::frame::FrameBudget::observe",
           "if byte == b'\\n' {", "if byte == b'\\r' {"),
    Mutant("process-output-counter-reset", "process_budget/output.rs", "process_budget::output::ObservedBytes::record",
           "self.total += bytes;", "self.total = bytes;"),
    Mutant("process-tail-prefix", "process_budget/tail.rs", "process_budget::tail::retain_tail",
           "let old_room = capacity - incoming;", "let old_room = capacity;"),
    Mutant("process-input-window", "process_budget/transfer.rs", "process_budget::transfer::InputTransfer::begin",
           "let end = self.sent + bytes;", "let end = bytes;"),
    Mutant("process-input-retry-reset", "process_budget/transfer.rs", "process_budget::transfer::InputTransfer::retry",
           "if matches!(self.phase, TransferPhase::Offered(_)) { self.phase = TransferPhase::Ready; }",
           "if matches!(self.phase, TransferPhase::Offered(_)) { self.sent = 0; self.phase = TransferPhase::Ready; }"),
    Mutant("process-input-overacknowledge", "process_budget/transfer.rs", "process_budget::transfer::InputTransfer::complete",
           "if written == 0 || written > bytes {", "if written == 0 {"),
    Mutant("process-lost-ownership", "process_budget/lifecycle.rs", "process_budget::lifecycle::ChildLifecycle::lose_ownership",
           "{ self.leader = LeaderState::Lost; }", "{ self.leader = LeaderState::Running; }"),
    Mutant("process-reap-before-signal", "process_budget/lifecycle.rs", "process_budget::lifecycle::ChildLifecycle::begin_reap",
           "if self.finished || !matches!(self.leader, LeaderState::Exited) || matches!(self.group, GroupSignalState::Pending) { return false; }",
           "if self.finished || !matches!(self.leader, LeaderState::Exited) { return false; }"),
    Mutant("process-reaped-ownership", "process_budget/lifecycle.rs", "process_budget::lifecycle::ChildLifecycle::observe_reap",
           "ReapObservation::Reaped => LeaderState::Reaped,", "ReapObservation::Reaped => LeaderState::Exited,"),
    Mutant("process-expired-cleanup-success", "process_budget/lifecycle.rs", "process_budget::lifecycle::ChildLifecycle::cleanup_decision",
           "        if !budget_remaining { CleanupDecision::Deadline }",
           "        if !budget_remaining && !(matches!(self.leader, LeaderState::Reaped) && drained) { CleanupDecision::Deadline }"),
    Mutant("process-finished-revival", "process_budget/lifecycle.rs", "process_budget::lifecycle::ChildLifecycle::finish",
           "self.finished = true;", "self.finished = false;"),
    Mutant("survey-report-position", "update_survey.rs", "update_survey::SurveyProgress::record",
           "self.invalid || reported != counted || counted == self.selected",
           "self.invalid || counted == self.selected"),
    Mutant("survey-selected-bound", "update_survey.rs", "update_survey::SurveyProgress::record",
           "self.invalid || reported != counted || counted == self.selected",
           "self.invalid || reported != counted"),
    Mutant("survey-failure-accounting", "update_survey.rs", "update_survey::SurveyProgress::record",
           "UpdateDisposition::Failed => self.failed += 1,", "UpdateDisposition::Failed => self.skipped += 1,"),
    Mutant("survey-invalid-revival", "update_survey.rs", "update_survey::SurveyProgress::record",
           "self.invalid = true;", "self.invalid = false;"),
    Mutant("survey-partial-finish", "update_survey.rs", "update_survey::SurveyProgress::finish",
           "self.invalid || reported != self.selected || self.counted_reports() != self.selected",
           "self.invalid || reported != self.selected"),
    Mutant("survey-error-precedence", "update_survey.rs", "update_survey::UpdateSummary::outcome",
           "if self.counts.failed != 0 { UpdateCheckOutcome::Incomplete }",
           "if self.counts.failed != 0 && self.counts.changed == 0 { UpdateCheckOutcome::Incomplete }"),
    Mutant("survey-check-publication", "update_survey.rs", "update_survey::UpdateSummary::publishes_lock",
           "{ mode == UpdateMode::Publish && self.counts.failed == 0 && self.counts.changed != 0 }",
           "{ mode == UpdateMode::Check && self.counts.failed == 0 && self.counts.changed != 0 }"),
    Mutant("survey-unchanged-publication", "update_survey.rs", "update_survey::UpdateSummary::publishes_lock",
           "{ mode == UpdateMode::Publish && self.counts.failed == 0 && self.counts.changed != 0 }",
           "{ mode == UpdateMode::Publish && self.counts.failed == 0 }"),
    Mutant("journal-publication-file-barrier", "journal_protocol.rs", "journal_protocol::RecordPublication::acknowledge",
           "self.next = match action {\n            PublicationAction::Stage => PublicationAction::SyncFile,",
           "self.next = match action {\n            PublicationAction::Stage => PublicationAction::PublishName,"),
    Mutant("journal-publication-failed-ack", "journal_protocol.rs", "journal_protocol::RecordPublication::acknowledge",
           "if !succeeded || action != self.next {", "if action != self.next {"),
    Mutant("journal-publication-early-authority", "journal_protocol.rs", "journal_protocol::RecordPublication::finish",
           "if self.next == PublicationAction::Complete { Some(DurableRecord { role: self.role }) }",
           "if self.next == PublicationAction::PublishName { Some(DurableRecord { role: self.role }) }"),
    Mutant("journal-mutation-role-confusion", "journal_protocol.rs", "journal_protocol::admit_mutation",
           "if marker.role == RecordRole::RunMarker && entry.role == RecordRole::MutationEntry {",
           "if marker.role == RecordRole::RunMarker || entry.role == RecordRole::MutationEntry {"),
    Mutant("journal-cleanup-uncommitted-authority", "journal_protocol.rs", "journal_protocol::CleanupProgress::new",
           "reconciled: if scope == CleanupScope::Committed { total } else { 0 },",
           "reconciled: total,"),
    Mutant("journal-cleanup-action-guard", "journal_protocol.rs", "journal_protocol::CleanupProgress::acknowledge",
           "if !succeeded || action != self.action() {", "if !succeeded {"),
    Mutant("journal-cleanup-entry-accounting", "journal_protocol.rs", "journal_protocol::CleanupProgress::acknowledge",
           "CleanupAction::RemoveEntry { index: _ } => { self.removed += 1; }",
           "CleanupAction::RemoveEntry { index: _ } => { self.removed = self.total; }"),
    Mutant("journal-cleanup-marker-barrier", "journal_protocol.rs", "journal_protocol::CleanupProgress::acknowledge",
           "CleanupAction::SyncEntries => { self.phase = CleanupPhase::MarkerRemoval; }",
           "CleanupAction::SyncEntries => { self.phase = CleanupPhase::MarkerBarrier; }"),
    # semantic normalization: framing header corruption
    Mutant("semantic-frame-length", "semantic.rs", "semantic::length_header",
           "        let byte = ((length >> shift) & 0xffu64) as u8;",
           "        let byte = ((length >> shift) & 0xfeu64) as u8;"),
    # semantic normalization: never place before a greater entry (order loss)
    Mutant("semantic-canonical-position", "semantic.rs", "semantic::canonical_entries",
           "            if entry_leq(entry, result[j]) {",
           "            if false {"),
    # semantic normalization: drop members instead of duplicates
    Mutant("semantic-canonical-dedup", "semantic.rs", "semantic::canonical_entries",
           "        if j < result.len() && entry_equal(entry, result[j]) {",
           "        if j < result.len() && !entry_equal(entry, result[j]) {"),
    # semantic normalization: ignore the secondary key (lock scope conflation)
    Mutant("semantic-pair-order", "semantic.rs", "semantic::entry_leq",
           "        bytes_leq(a.1, b.1)",
           "        true"),
    # identity boundary: provenance enters identity (A1-04 calibration)
    Mutant("semantic-provenance-boundary", "semantic.rs", "semantic::lemma_provenance_cannot_change_identity",
           "    encode_frames(view.semantics)",
           "    encode_frames(view.semantics) + encode_frames(view.provenance)",
           also_fails=("semantic::lemma_semantic_change_invalidates_identity",)),
    # label/module/import order becomes observable (A1-03/A1-04 calibration)
    Mutant("semantic-unordered-canonical", "semantic.rs", "semantic::lemma_unordered_presentation_invariant",
           "    seq![le64(canonical_form(values).len() as u64)] + entry_frames(canonical_form(values))",
           "    seq![le64(values.len() as u64)] + entry_frames(values)",
           also_fails=("semantic::lemma_unordered_content_injective",)),
    # projection coverage: reversed witness path (dropped-coverage calibration)
    Mutant("graph-projection-coverage", "graph.rs", "graph::lemma_projected_build_edge_in_closure",
           "    assert(is_path(adjacency, seq![from, to], from, to));",
           "    assert(is_path(adjacency, seq![to, from], from, to));"),
)


def verify(crate: Path, target: Path) -> tuple[int, Evidence]:
    package = crate.name
    command = [
        "cargo", "verus", "verify", "--manifest-path", str(crate / "Cargo.toml"),
        "-p", package, "--locked", "--message-format=json", "--",
        "--output-json", "--time-expanded",
    ]
    print("RUNNER_COMMAND=" + " ".join(command), flush=True)
    result = subprocess.run(
        command, cwd=crate, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
        text=True, timeout=600, env={**os.environ, "CARGO_TARGET_DIR": str(target)},
    )
    try:
        evidence = Evidence.parse(result.stdout, package.replace("-", "_"))
    except EvidenceError:
        print(result.stdout, flush=True)
        raise
    print("VERUS_RESULT=" + json.dumps(evidence.results, sort_keys=True), flush=True)
    for diagnostic in evidence.diagnostics:
        if diagnostic.get("level") == "error":
            print("VERUS_DIAGNOSTIC=" + json.dumps(diagnostic, sort_keys=True), flush=True)
    return result.returncode, evidence


def copy_crate(destination: Path, package: str = "gripsack-policy") -> Path:
    # Preserve the complete workspace and its exact lock graph. Making a
    # standalone package changes Cargo's root lock entry and can silently
    # re-resolve transitive tools; mutants change only the selected crate's source.
    destination.mkdir()
    for name in ("Cargo.toml", "Cargo.lock"):
        shutil.copy2(ROOT / name, destination / name)
    for name in ("crates", "fuzz", "schema"):
        shutil.copytree(ROOT / name, destination / name, symlinks=True,
                        ignore=shutil.ignore_patterns("target", "__pycache__"))
    return destination / "crates" / package


def main() -> None:
    self_check()
    with tempfile.TemporaryDirectory(prefix="gripsack-verus-") as directory:
        temporary = Path(directory)
        status, evidence = verify(CRATE, temporary / "positive-target")
        evidence.positive(status, MIN_OBLIGATIONS, FAMILIES)
        print(f"positive: verification results:: {evidence.results['verified']} verified, 0 errors", flush=True)
        for family, required in FAMILIES.items():
            print(f"VERUS_FAMILY={family} checked={len(required)} functions={','.join(required)}", flush=True)
        for mutation in MUTANTS:
            crate = copy_crate(temporary / mutation.name)
            path = crate / "src" / mutation.file
            source = path.read_text()
            if source.count(mutation.before) != 1:
                raise EvidenceError(f"{mutation.name}: source mutation does not uniquely match")
            path.write_text(source.replace(mutation.before, mutation.after))
            status, evidence = verify(crate, temporary / (mutation.name + "-target"))
            expected = (mutation.function, *mutation.also_fails)
            evidence.mutant(status, path, expected, crate.parent.parent)
            print(f"calibration: {mutation.name} rejected in {','.join(expected)}", flush=True)
        # A real unrelated failing lemma in the same source file cannot
        # impersonate the classifier mutant's named production failure.
        crate = copy_crate(temporary / "unrelated-lemma")
        path = crate / "src/selection.rs"
        source = path.read_text()
        prefix, suffix = source.rsplit("}", 1)
        path.write_text(prefix + "pub proof fn unrelated_calibration_failure() ensures false {}\n}" + suffix)
        status, evidence = verify(crate, temporary / "unrelated-target")
        evidence.mutant(status, path, ("selection::unrelated_calibration_failure",), crate.parent.parent)
        try:
            evidence.mutant(status, path, ("selection::classify",), crate.parent.parent)
        except EvidenceError:
            print("calibration: unrelated lemma refused as classifier evidence", flush=True)
        else:
            raise EvidenceError("unrelated lemma falsely calibrated classifier")
        print(f"VERUS_CALIBRATIONS={len(MUTANTS) + 1}", flush=True)
        from check_collector_verus import check as check_collector
        check_collector(temporary)
        print("verify gate: OK", flush=True)


if __name__ == "__main__":
    try:
        main()
    except (EvidenceError, subprocess.TimeoutExpired) as error:
        raise SystemExit(f"FAIL: {error}")
