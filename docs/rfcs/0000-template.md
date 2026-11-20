---
rfc: "YYYY-MM-DD-kebab-title"
title: "Short descriptive title"
status: draft
implementation: not-started
authors:
  - handle
created: YYYY-MM-DD
updated: YYYY-MM-DD
issue: null
milestone: null
supersedes: []
superseded_by: []
blocked_on: []
---

# RFC: Short descriptive title

Copy this file to `YYYY-MM-DD-kebab-title.md`, fill in the tracking issue URL and other metadata, and add the draft to the [registry](README.md). Use the registry's lifecycle and field meanings. Replace the prompts with answers; leave the milestone `null` unless one has been assigned.

## Summary and motivation

State the decision, the problem it solves, the evidence that the problem exists, and what is outside the decision. Why is an issue alone not enough?

## Observable behaviour

Show an example from a workspace author's or operator's point of view: commands, outputs, files, refusals, exit status, or recovery. Distinguish before from after.

## Proof obligations

For a ring 0 decision or ring 1 sans-IO protocol, name the shipped Rust kernel entry points and their inputs and outputs. State each Lean theorem over the Aeneas translation, which mutant should break it, and which ring 2 host assumptions and ring 3 input checks it relies on. Identify what remains tested, not proved. If no proved behaviour changes, say why.

## Design and boundaries

Name the owning crates and rings, the direction of dependencies, admitted data, and who can cause effects. For a protocol, specify ordering, interruption and recovery. For an IR change, cover schema, Rust admission, and TypeScript emission together.

## Compatibility and reversibility

What does an existing workspace, lockfile, store or retained generation observe? Cover reads, writes, rollback and limits of compatibility where relevant. Explain the cost of undoing this decision.

## Alternatives

Compare the strongest alternative, including leaving the behaviour unchanged. What evidence would change the choice?

## Evidence and rollout

List the proof, real-binary fixture, regression test or other check that would establish each claim. Describe independently safe implementation steps, dependencies, and the observable finish line. Link the issues for split work; do not guess a release date.

## Unresolved questions

List the questions that must be answered before acceptance. Identify the owner decision needed and reflect blockers in `blocked_on`.

## Decision log

Record dated owner decisions, rationale and material amendments. On implementation, note any differences from the accepted design and update the frontmatter and registry. A merged draft is not owner acceptance.
