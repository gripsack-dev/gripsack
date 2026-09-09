use crate::ctx::{Ctx, ExecError};
use crate::lockfile::LockEntry;
use crate::source::preflight::LayoutEvidence;
use gripsack_ir::prepared::PreparedModule;

pub(super) struct PreparedUpdate {
    pub entry: LockEntry,
    pub layout: LayoutEvidence,
    stage: tempfile::TempDir,
    cache: Option<std::path::PathBuf>,
}

impl PreparedUpdate {
    pub fn acquire(ctx: &Ctx, name: &str, plan: &PreparedModule) -> Result<Self, ExecError> {
        let stage = tempfile::Builder::new().prefix("grip-update-").tempdir()?;
        let mut entry = crate::source::fetch(
            ctx,
            crate::source::FetchInputs {
                name,
                spec: plan.fetch().expect("caller selected a source"),
                locked: None,
                dest: stage.path(),
            },
        )?;
        let pin = entry.resolved.as_mut().expect("acquisition creates a pin");
        let overlay = crate::source::Overlay::capture(plan, &ctx.repo, stage.path())?;
        pin.repo256 = if plan.has_recipe() {
            overlay.into_hash()
        } else {
            overlay.merge(stage.path())?
        };
        let layout = crate::source::preflight::inspect(
            name,
            plan,
            crate::source::preflight::PayloadStage::AcquiredSource(stage.path()),
            pin.version.as_deref(),
        )?;
        let cache = if plan.has_recipe() {
            None
        } else {
            let tree = gripsack_store::canonical_tree_hash(stage.path())?.to_string();
            let destination = gripsack_store::content_path(&ctx.home, name, &tree);
            if destination.exists() {
                let actual = gripsack_store::canonical_tree_hash(&destination)?;
                if actual.as_str() != tree {
                    return Err(ExecError::Fetch(gripsack_fetch::FetchError::HashMismatch {
                        url: destination.display().to_string(),
                        expected: tree,
                        actual: actual.into(),
                    }));
                }
            }
            pin.tree256 = Some(tree);
            Some(destination)
        };
        Ok(Self {
            entry,
            stage,
            cache,
            layout,
        })
    }

    pub fn publish(&self, ctx: &Ctx, name: &str) -> Result<(), ExecError> {
        if let Some(destination) = &self.cache
            && !destination.exists()
        {
            crate::source::publish(ctx, name, self.stage.path(), destination)?;
        }
        Ok(())
    }
}
