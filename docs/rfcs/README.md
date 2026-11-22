# Gripsack RFCs

Start with a [GitHub issue](https://github.com/gripsack-dev/gripsack/issues). Issues track the problem, work and closure; [milestones](https://github.com/gripsack-dev/gripsack/milestones) group accepted work by release, or `Later` when no release is assigned. An RFC records a design and why it was chosen. Neither an issue nor an RFC is a release receipt.

## When to write an RFC

Use an RFC for a user-visible CLI or SDK contract; an IR, lockfile, store or retained-data format; a change in authority, ownership, rollback or recovery; new kernel decision logic or storage assumptions; or a dependency, platform or release policy that would be costly to undo. An ordinary bug fix that restores the existing contract usually needs only an issue and a regression witness. See [Contributing](../../CONTRIBUTING.md) and [Governance](../../GOVERNANCE.md), including the [private security reporting route](../../SECURITY.md).

## Propose and decide

1. Find or open the issue. The owner triages it; `needs-rfc` means the design must be reviewed before implementation.
2. Copy [0000-template.md](0000-template.md) to `YYYY-MM-DD-kebab-title.md`. Use the drafting date, not a new serial number; keep the filename on retitle. Link the issue both ways, apply `rfc` and add a registry row below.
3. Put proof obligations **before** the design for kernel or protocol changes: the shipped Rust decision, the Lean theorem over its Aeneas translation, the mutant that should break it, and what is merely assumed or tested at the host boundary. An RFC cannot waive the [engineering and release rules](../../AGENTS.md).
4. Only the owner accepts or rejects a proposal. Merging a draft is not acceptance. Record the decision and rationale in the dated decision log. Keep rejected and superseded records; link replacements in both directions.
5. Implementation PRs use `Refs #N`, or `Closes #N` for the final delivered scope. Update the RFC's implementation state, as-built notes and registry row in that PR. Split independent work into linked issues; do not keep a second backlog here.

## Frontmatter

| Field | Meaning |
|---|---|
| `rfc`, `title` | Filename stem and heading text (`# RFC: Title`). |
| `status` | `draft`, `accepted`, `rejected` or `superseded`. A replacement must be accepted before it supersedes a decision. |
| `implementation` | `not-started`, `in-progress`, `partial`, `complete`, `removed` or `n/a`. Describes the agreed scope, not whether the document was merged. |
| `authors`, `created`, `updated` | Authors, drafting date and last material revision (`YYYY-MM-DD`). |
| `issue`, `milestone` | Tracking issue URL and its milestone title; `null` when no milestone is assigned. |
| `supersedes`, `superseded_by`, `blocked_on` | Lists of RFC IDs, or for blockers, issue URLs or specific owner questions. Several successors may jointly replace a roadmap. A section-level amendment is described in the text, not marked as a whole-record replacement. |
| `legacy_id` | Backfilled records only: the original numbered citation, including `0056a`. Never allocate a new one. |

Frontmatter is authoritative for RFC status and implementation. GitHub is authoritative for issue state and scheduling. A closed historical issue can have `implementation: partial` or `removed`: closure does not assert the remainder shipped.

## History and current work

This workflow follows [writfold's adoption of Omnigraph's RFC process](https://github.com/writfold-dev/writfold/commit/640e0fbd10794fc0962b8262d5a6648497d99fd9). On 2026-11-20, Gripsack imported all 60 designs from [the last pre-migration tree](https://github.com/gripsack-dev/gripsack/tree/7924bc2b844d109dbb520040f5954ad237007ff3/plan). The [final ledger](legacy-ledger.md) is a frozen source, not a live checklist. `0034+` was the VM harness follow-up to 0034 ([PR #122](https://github.com/gripsack-dev/gripsack/pull/122)), not a missing RFC.

The old headings, status lines, promises, release numbers and paths in historical prose describe their time. Each RFC links its exact original and its tracking issue, including later changes of direction. Numeric citations such as `plan/0002 §4` in historical commits, code comments and versioned schemas resolve by **Legacy** below. For an old revision, use the original path in that revision. Import dates on GitHub are not historical acceptance or completion dates. The drafting date is the document's stated date where available, otherwise the first recorded Git addition. The credited Git author is not a claim of sole authorship.

Start with the [verified foundation (#306)](https://github.com/gripsack-dev/gripsack/issues/306) for remaining storage proofs and the [front door (#310)](https://github.com/gripsack-dev/gripsack/issues/310) for IR11 and daily-use commands. The [handover reconciliation (#309)](https://github.com/gripsack-dev/gripsack/issues/309) links the surviving follow-ups; its closed state is for the review, not those changes.

## Registry

Creation order; new proposals use `—` in the Legacy column.

| Legacy | RFC | Decision | Implementation | Tracking issue |
|---|---|---|---|---|
| <a id="legacy-0001"></a>0001 | [User-scoped store, ownership and generations](2026-08-22-architecture.md) | accepted | complete | [#251](https://github.com/gripsack-dev/gripsack/issues/251) |
| <a id="legacy-0002"></a>0002 | [Source resolution, transport and locked payload paths](2026-08-22-sourcing.md) | accepted | complete | [#252](https://github.com/gripsack-dev/gripsack/issues/252) |
| <a id="legacy-0003"></a>0003 | [Coordinated repository, qualification and release policy](2026-08-22-repo-and-tooling.md) | accepted | complete | [#253](https://github.com/gripsack-dev/gripsack/issues/253) |
| <a id="legacy-0004"></a>0004 | [Source-mapped IR and compiler diagnostics](2026-08-22-rich-ir-and-passes.md) | accepted | complete | [#254](https://github.com/gripsack-dev/gripsack/issues/254) |
| <a id="legacy-0005"></a>0005 | [Frontend contract and layered configuration](2026-08-22-frontends-and-configuration.md) | accepted | partial | [#255](https://github.com/gripsack-dev/gripsack/issues/255) |
| <a id="legacy-0006"></a>0006 | [Gradual adoption alongside existing package managers](2026-08-22-gradual-migration.md) | accepted | complete | [#256](https://github.com/gripsack-dev/gripsack/issues/256) |
| <a id="legacy-0007"></a>0007 | [Step dependencies, resources and scheduling](2026-08-22-steps-and-scheduling.md) | accepted | partial | [#257](https://github.com/gripsack-dev/gripsack/issues/257) |
| <a id="legacy-0008"></a>0008 | [Canonical content identity and no-op apply](2026-08-22-canonicalization-and-satisfaction.md) | accepted | complete | [#258](https://github.com/gripsack-dev/gripsack/issues/258) |
| <a id="legacy-0009"></a>0009 | [Structured diagnostics across process boundaries](2026-08-23-diagnostics-across-boundaries.md) | accepted | complete | [#259](https://github.com/gripsack-dev/gripsack/issues/259) |
| <a id="legacy-0010"></a>0010 | [Provision Python plugins in the frontend environment](2026-08-24-plugin-provisioning.md) | superseded | removed | [#260](https://github.com/gripsack-dev/gripsack/issues/260) |
| <a id="legacy-0011"></a>0011 | [Frontend-driven validation plugins](2026-08-24-validation-plugins.md) | superseded | removed | [#261](https://github.com/gripsack-dev/gripsack/issues/261) |
| <a id="legacy-0012"></a>0012 | [Core-driven linting and managed plugin executables](2026-08-26-linters-in-core.md) | accepted | partial | [#262](https://github.com/gripsack-dev/gripsack/issues/262) |
| <a id="legacy-0013"></a>0013 | [Constrained TypeScript evaluation and explicit trust](2026-08-28-constrained-evaluation.md) | accepted | partial | [#263](https://github.com/gripsack-dev/gripsack/issues/263) |
| <a id="legacy-0014"></a>0014 | [Content-addressed fetches and input-addressed builds](2026-08-28-content-addressed-fetches.md) | accepted | complete | [#264](https://github.com/gripsack-dev/gripsack/issues/264) |
| <a id="legacy-0015"></a>0015 | [Reversible adoption of existing dotfiles](2026-08-29-grip-adopt.md) | accepted | complete | [#265](https://github.com/gripsack-dev/gripsack/issues/265) |
| <a id="legacy-0016"></a>0016 | [Platform selectors, floating Git pins and read-only payloads](2026-08-29-platform-facts-floating-git-readonly-store.md) | accepted | complete | [#266](https://github.com/gripsack-dev/gripsack/issues/266) |
| <a id="legacy-0017"></a>0017 | [Repeatable hardening and fuzzing rounds](2026-09-02-hardening-fuzzing-playbook.md) | accepted | complete | [#267](https://github.com/gripsack-dev/gripsack/issues/267) |
| <a id="legacy-0018"></a>0018 | [0.17.14 migration and runtime review response](2026-09-02-critique-response-0-17-14.md) | accepted | complete | [#268](https://github.com/gripsack-dev/gripsack/issues/268) |
| <a id="legacy-0019"></a>0019 | [Write-ahead journal for deploy crash recovery](2026-09-02-deploy-journal.md) | accepted | complete | [#269](https://github.com/gripsack-dev/gripsack/issues/269) |
| <a id="legacy-0020"></a>0020 | [Post-flip recovery and 0.18.1 architecture review](2026-09-03-review-response.md) | accepted | complete | [#270](https://github.com/gripsack-dev/gripsack/issues/270) |
| <a id="legacy-0021"></a>0021 | [Capability-relative filesystem writes](2026-09-04-cap-std-fs-hardening.md) | accepted | complete | [#271](https://github.com/gripsack-dev/gripsack/issues/271) |
| <a id="legacy-0022"></a>0022 | [Embed the release binary's software bill of materials](2026-09-04-sbom-cargo-auditable.md) | accepted | complete | [#272](https://github.com/gripsack-dev/gripsack/issues/272) |
| <a id="legacy-0023"></a>0023 | [Reject case-variant duplicate destinations on every host](2026-09-04-case-fold-everywhere.md) | accepted | complete | [#273](https://github.com/gripsack-dev/gripsack/issues/273) |
| <a id="legacy-0024"></a>0024 | [Fixes from the 0.21 migration report](2026-09-04-review-response-0-21-0.md) | accepted | complete | [#274](https://github.com/gripsack-dev/gripsack/issues/274) |
| <a id="legacy-0025"></a>0025 | [Transactional coverage for rollback, prune and profiles](2026-09-04-transaction-coverage.md) | accepted | complete | [#275](https://github.com/gripsack-dev/gripsack/issues/275) |
| <a id="legacy-0026"></a>0026 | [Generation identity and path-centric transactions](2026-09-04-path-centric-transactions.md) | accepted | complete | [#276](https://github.com/gripsack-dev/gripsack/issues/276) |
| <a id="legacy-0027"></a>0027 | [Verified postconditions and admitted generations](2026-09-04-provable-transactions.md) | accepted | complete | [#277](https://github.com/gripsack-dev/gripsack/issues/277) |
| <a id="legacy-0028"></a>0028 | [Machine-checked transaction model](2026-09-05-machine-checked-model.md) | accepted | complete | [#278](https://github.com/gripsack-dev/gripsack/issues/278) |
| <a id="legacy-0029"></a>0029 | [Ownership lineage and authorized transitions](2026-09-05-ownership-lineage.md) | accepted | complete | [#279](https://github.com/gripsack-dev/gripsack/issues/279) |
| <a id="legacy-0030"></a>0030 | [Canonical destinations and composed authorization](2026-09-05-canonical-destinations.md) | accepted | complete | [#280](https://github.com/gripsack-dev/gripsack/issues/280) |
| <a id="legacy-0031"></a>0031 | [File mode as part of object identity](2026-09-05-mode-aware-identity.md) | accepted | complete | [#281](https://github.com/gripsack-dev/gripsack/issues/281) |
| <a id="legacy-0032"></a>0032 | [Durable activation hooks](2026-09-05-durable-activation.md) | accepted | complete | [#282](https://github.com/gripsack-dev/gripsack/issues/282) |
| <a id="legacy-0033"></a>0033 | [Findings from the 0.27 external review](2026-09-05-review-response-0-27-0.md) | accepted | complete | [#283](https://github.com/gripsack-dev/gripsack/issues/283) |
| <a id="legacy-0034"></a>0034 | [One operation list for plan, apply and rollback](2026-09-05-shared-operation-list.md) | accepted | complete | [#284](https://github.com/gripsack-dev/gripsack/issues/284) |
| <a id="legacy-0035"></a>0035 | [Destination identity and transaction boundary review](2026-09-06-review-fb11aaf.md) | accepted | complete | [#285](https://github.com/gripsack-dev/gripsack/issues/285) |
| <a id="legacy-0036"></a>0036 | [Audit of domain types and wire representations](2026-09-06-representation-audit.md) | accepted | complete | [#286](https://github.com/gripsack-dev/gripsack/issues/286) |
| <a id="legacy-0037"></a>0037 | [Activate the selected generation on rollback](2026-09-06-rollback-activation.md) | accepted | complete | [#287](https://github.com/gripsack-dev/gripsack/issues/287) |
| <a id="legacy-0038"></a>0038 | [Stateful journey testing](2026-09-06-journey-harness.md) | accepted | complete | [#288](https://github.com/gripsack-dev/gripsack/issues/288) |
| <a id="legacy-0039"></a>0039 | [Build-only dependency closures](2026-09-06-build-closures.md) | superseded | removed | [#289](https://github.com/gripsack-dev/gripsack/issues/289) |
| <a id="legacy-0040"></a>0040 | [Post-0.35 contract and persistence review](2026-09-07-project-sweep.md) | accepted | complete | [#290](https://github.com/gripsack-dev/gripsack/issues/290) |
| <a id="legacy-0041"></a>0041 | [P1 contract fidelity and persistence repairs](2026-09-07-p1-contract-fidelity.md) | accepted | complete | [#291](https://github.com/gripsack-dev/gripsack/issues/291) |
| <a id="legacy-0042"></a>0042 | [Bounded acquisition and complete lock pins](2026-09-07-bounded-acquisition-and-complete-pins.md) | accepted | complete | [#292](https://github.com/gripsack-dev/gripsack/issues/292) |
| <a id="legacy-0043"></a>0043 | [Executable templates and truthful update checks](2026-09-07-migration-feedback-0-37.md) | accepted | complete | [#293](https://github.com/gripsack-dev/gripsack/issues/293) |
| <a id="legacy-0044"></a>0044 | [Complete update surveys and bounded HTTP failures](2026-09-09-survey-paths-and-transport-feedback.md) | accepted | complete | [#294](https://github.com/gripsack-dev/gripsack/issues/294) |
| <a id="legacy-0045"></a>0045 | [Recovery admission and editor-visible frontend types](2026-09-10-recovery-admission-and-editor-frontend.md) | accepted | complete | [#295](https://github.com/gripsack-dev/gripsack/issues/295) |
| <a id="legacy-0046"></a>0046 | [Verified decision kernels: the Verus pilot](2026-09-10-verified-policy-kernels.md) | superseded | removed | [#296](https://github.com/gripsack-dev/gripsack/issues/296) |
| <a id="legacy-0047"></a>0047 | [Verified merge splice, graph closure and scheduling](2026-09-10-verified-merge-and-scheduling.md) | accepted | removed | [#297](https://github.com/gripsack-dev/gripsack/issues/297) |
| <a id="legacy-0048"></a>0048 | [Harden the evaluator, archives, recovery and release boundaries](2026-09-24-review-response-0-42-0.md) | accepted | partial | [#298](https://github.com/gripsack-dev/gripsack/issues/298) |
| <a id="legacy-0049"></a>0049 | [Import the edition-five handover and reconcile its delivery inventory](2026-09-24-handover-bundle-import.md) | superseded | partial | [#299](https://github.com/gripsack-dev/gripsack/issues/299) |
| <a id="legacy-0050"></a>0050 | [Qualify finite tasks and user scheduling](2026-09-24-e0-qualification.md) | accepted | partial | [#300](https://github.com/gripsack-dev/gripsack/issues/300) |
| <a id="legacy-0051"></a>0051 | [Qualify Linux BuildKit workers and the LLB bridge](2026-09-24-b0-qualification.md) | superseded | removed | [#301](https://github.com/gripsack-dev/gripsack/issues/301) |
| <a id="legacy-0052"></a>0052 | [Define typed workspaces, graphs and native file profiles](2026-09-24-workspace-contract.md) | accepted | complete | [#302](https://github.com/gripsack-dev/gripsack/issues/302) |
| <a id="legacy-0053"></a>0053 | [Move release qualification to exact-source local builds](2026-10-04-release-ci-research-handoff.md) | accepted | complete | [#303](https://github.com/gripsack-dev/gripsack/issues/303) |
| <a id="legacy-0054"></a>0054 | [Simplify workspace authoring and the reviewer migration path](2026-10-24-workspace-ergonomics-and-handover-completion.md) | accepted | complete | [#304](https://github.com/gripsack-dev/gripsack/issues/304) |
| <a id="legacy-0055"></a>0055 | [Make native workspaces the only product surface](2026-10-27-workspace-only-product.md) | accepted | complete | [#305](https://github.com/gripsack-dev/gripsack/issues/305) |
| <a id="legacy-0056"></a>0056 | [Build a Lean-proved transaction kernel and explicit storage protocols](2026-11-08-verified-foundation.md) | accepted | in-progress | [#306](https://github.com/gripsack-dev/gripsack/issues/306), [leaf P1](https://github.com/gripsack-dev/gripsack/issues/365) |
| <a id="legacy-0056a"></a>0056a | [Reconcile the original handover against the 0.51 tree](2026-11-08-handover-reconciliation.md) | superseded | complete | [#307](https://github.com/gripsack-dev/gripsack/issues/307) |
| <a id="legacy-0057"></a>0057 | [Shorten qualification without dropping gates](2026-11-08-fast-qualification.md) | accepted | complete | [#308](https://github.com/gripsack-dev/gripsack/issues/308) |
| <a id="legacy-0058"></a>0058 | [Close the historical handover and identify surviving work](2026-11-09-handover-final.md) | accepted | complete | [#309](https://github.com/gripsack-dev/gripsack/issues/309) |
| <a id="legacy-0059"></a>0059 | [Simplify approval, workspace syntax and everyday commands](2026-11-09-front-door.md) | accepted | in-progress | [#310](https://github.com/gripsack-dev/gripsack/issues/310) |
