//! Module identity (0014 §3, 0008 §5): which store path names a
//! module's content — decided before execution, from the module's
//! recipe, its lock pin, and the repo's current sources. Pure
//! computation, no phase state: the phase machine (module.rs)
//! consumes the answer.

use crate::ctx::{Ctx, ExecError};
use crate::lockfile;
use crate::report::{ReportKind, StepReport};
use crate::resolve::module_input;
use gripsack_ir::step::{Step, StepAction};
use gripsack_ir::{Ir, Module};
use gripsack_store as store;
use std::path::PathBuf;

/// A module's resolved identity: where its content lives (or will
/// live), how that path was named, and whether it is already there.
pub(crate) struct ModuleIdentity {
    pub store_path: PathBuf,
    /// 0014 §3: no build/custom/run step → the store path names the
    /// content itself (tree hash), not the recipe.
    pub content_addressed: bool,
    /// Identity is finalized after fetch for kinds whose payload hash
    /// isn't knowable up front (pixi, git, plugin — finding C): the
    /// lock-independent path is provisional, and the first fetch's
    /// sha256 completes it — the same path every later apply computes
    /// from the lockfile. Content-addressed modules (0014) finalize at
    /// publish instead: the tree needs the merged staging.
    pub identity_pending: bool,
    /// The content identity: the expected tree hash from the lock (or
    /// the plan-time overlay for config-only), then the computed tree
    /// at publish. Recorded into the generation manifest for
    /// host-independent store verify.
    pub tree256: Option<String>,
    /// Satisfaction: the payload is already in the store.
    pub present: bool,
}

impl ModuleIdentity {
    /// The report a satisfied identity earns.
    pub(crate) fn satisfied_report(&self, name: &str) -> Option<StepReport> {
        self.present.then(|| StepReport {
            module: name.to_string(),
            summary: if self.content_addressed {
                "content already in store".into()
            } else {
                "payload already in store".into()
            },
            kind: ReportKind::Satisfied,
        })
    }
}

/// Resolve a module's identity and satisfaction: the payload hash
/// joins the store-path input before the existence check, so first
/// and second applies compute the same path (0008 §5).
pub(crate) fn resolve(
    name: &str,
    module: &Module,
    ir: &Ir,
    steps: &[Step],
    ctx: &Ctx,
    locked: Option<&lockfile::LockEntry>,
) -> Result<ModuleIdentity, ExecError> {
    // 0014 §3: content is fully determined before execution unless
    // a build/custom/run step exists. Fetches pin content via the
    // lock's tree256; config-only modules hash their repo sources
    // at plan time. Anything else is input-addressed (recipe-named,
    // plan-time-computable, not content-guaranteed).
    let content_addressed = !steps.iter().any(|s| {
        matches!(
            s.action,
            StepAction::Build { .. } | StepAction::CustomShell { .. } | StepAction::Run { .. }
        )
    });
    // the fetch spec lives in module.fetch (declarative) or in a
    // fetch step (explicit steps) — one source of truth for identity
    // and the lockfile resolver alike
    let fetch_spec = fetch_spec(module, steps);
    // the recipe left the path, so one re-fetch must PROVE byte
    // identity — publish dedups if it matches (the mirror swap)
    let spec_changed = match (locked, fetch_spec) {
        (Some(entry), Some(spec)) => entry.fetch != *spec,
        _ => false,
    };
    // Repo-overlay drift: the locked tree256 names the MERGED
    // staging (fetch payload + repo config files), so a config tree
    // that gains a file moves nothing the transport pin can see.
    // Compare the overlay half or a warm store would deploy stale
    // content. Locks predating repo256 with repo-sourced froms
    // drift once and heal at the next publish.
    let pinned = locked.and_then(|e| e.resolved.as_ref());
    let repo_drift = !spec_changed
        && pinned.is_some_and(|r| r.tree256.is_some())
        && match (
            crate::resolve::repo_overlay(module, &ctx.repo)?,
            pinned.and_then(|r| r.repo256.as_ref()),
        ) {
            (Some(current), Some(lock)) => current != *lock,
            // an old lock can't vouch for the overlay — distrust
            (Some(_), None) => true,
            (None, _) => false,
        };
    let (store_path, identity_pending, tree256) = if content_addressed {
        let locked_tree = if spec_changed || repo_drift {
            None
        } else {
            locked
                .and_then(|e| e.resolved.as_ref())
                .and_then(|r| r.tree256.clone())
        };
        match locked_tree {
            Some(tree) => (
                store::content_path(&ctx.home, name, &tree),
                false,
                Some(tree),
            ),
            None if fetch_spec.is_none() => {
                // config-only: content is the repo's payload sources,
                // computable without staging (overlay == staged tree)
                let froms: Vec<String> = module
                    .install
                    .iter()
                    .chain(module.config.iter())
                    .map(|e| e.from.clone())
                    .collect();
                let tree = store::canonical_overlay_hash(&ctx.repo, &froms)?.to_string();
                (
                    store::content_path(&ctx.home, name, &tree),
                    false,
                    Some(tree),
                )
            }
            None => {
                // deferred: the transport hash cannot name an
                // unextracted tree — the first fetch finalizes the
                // path at publish (0002 §3 TOFU)
                let input = module_input(module, &ctx.repo, ir)?;
                (store::store_path(&ctx.home, name, &input), true, None)
            }
        }
    } else {
        let resolved = locked
            .and_then(|e| e.resolved.as_ref())
            .and_then(|r| r.sha256.clone())
            .or_else(|| fetch_spec.and_then(|s| gripsack_fetch::payload_hash(s).ok().flatten()));
        let input = match &resolved {
            Some(sha) => format!("{}|payload={sha}", module_input(module, &ctx.repo, ir)?),
            None => module_input(module, &ctx.repo, ir)?,
        };
        let path = store::store_path(&ctx.home, name, &input);
        // Deferred identity (finding C): no hash from the lock AND
        // none computable offline — the first fetch's sha finalizes
        // the path. Presence is meaningless until then: always fetch.
        (path, resolved.is_none() && fetch_spec.is_some(), None)
    };
    let present = store_path.exists() && !identity_pending;
    Ok(ModuleIdentity {
        store_path,
        content_addressed,
        identity_pending,
        tree256,
        present,
    })
}

/// A module's fetch spec wherever it was declared: the declarative
/// `fetch` field, or the (single, E118-enforced) fetch step. The
/// lockfile resolver and the identity resolution must see the same
/// spec or steps-style modules pin differently than they resolve.
pub(crate) fn fetch_spec<'a>(
    module: &'a Module,
    steps: &'a [Step],
) -> Option<&'a gripsack_ir::FetchSpec> {
    module.fetch.as_ref().or_else(|| {
        steps.iter().find_map(|s| match &s.action {
            StepAction::Fetch { fetch } => Some(fetch),
            _ => None,
        })
    })
}
