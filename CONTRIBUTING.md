# Contributing to Gripsack

**Suspected vulnerability?** Follow [SECURITY.md](SECURITY.md) to report it privately. Do not put security details in a public issue or PR.

## Before writing code

1. Search the [issues](https://github.com/gripsack-dev/gripsack/issues) and [RFC registry](docs/rfcs/README.md). If the work is new, file a [bug report](https://github.com/gripsack-dev/gripsack/issues/new?template=bug_report.yml) for broken behaviour, or a [proposal](https://github.com/gripsack-dev/gripsack/issues/new?template=feature_request.yml) for a new capability or design change. Documentation issues can use either form; the owner can relabel them `documentation`.
2. Describe a concrete outcome. The owner triages the issue under [GOVERNANCE.md](GOVERNANCE.md). Wait for `accepted` before starting substantial implementation. A typo, broken link, or similarly obvious correction with no behaviour change may go directly to a PR.
3. If design is required, use the [RFC lifecycle and template](docs/rfcs/README.md), link the tracking issue both ways and add the registry row. You can submit a draft for review; only the owner can accept it. An `accepted` issue does not authorize implementing a draft RFC.

## Implement and qualify

Read [AGENTS.md](AGENTS.md) for working agreements. Gripsack targets Linux/WSL x86_64. Dependencies point inward: ring 0 makes pure decisions, ring 1 advances sans-IO protocol machines, ring 2 performs host effects under stated assumptions, and ring 3 admits untrusted input. Change the shipped kernel and its Lean proofs together; specify what stays tested rather than proved. A hand-written model is not a proof of the Rust.

For local iteration use `docker compose run --build --rm <service>` (for example `test`, `ts-test`, `e2e`, or `proofs`). The proofs gate checks Aeneas translation drift, builds the Lean proofs, audits axioms and runs mutant calibration. An IR change updates the schema, Rust types and TypeScript emitter together. Exercise a changed user flow with the real `grip` binary.

Protected `main` requires `local-practical-qualification` for the exact source on the owner's local Buildbot. [`.ci/pilot.json`](.ci/pilot.json) defines the required gates, including package and installed-consumer smoke checks. Local iteration and hosted diagnostics do not substitute for that status. External contributors need not operate the owner's Buildbot.

## Send the PR

- Describe the outcome, the accepted issue and any governing RFC. Use `Refs #N` in partial commits or PRs; use `Closes #N` in the finishing commit or PR only when it satisfies the issue. Don't close an implementation issue just because its RFC was merged.
- Show what you exercised and what happened (for example, before/after CLI output from a redacted fixture). Distinguish observed results from checks not run; don't commit logs as evidence.
- Update the affected documentation and [changelog](CHANGELOG.md) for user-visible changes. For RFC work, update its `implementation` field and registry row to match what actually landed. See the [PR skill](.agents/skills/gripsack-pr/SKILL.md) for the review checklist.
