//! Private staging outlives the complete solve and export verification. Only
//! native publication owns the durable, independently reconstructible cache.
use super::{failure, plan};
use crate::{
    Ctx, ExecError, LifecycleSession,
    workspace::{
        artifact::{self, Artifact},
        conda::{self, CondaDestination, CondaRuntimeReceipt},
        realize::Realization,
        selection::Selection,
    },
};
use gripsack_ir::workspace_v6::ImageOutput;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

const NAMESPACE: &str = "workspace-conda-image";
const RECEIPT: &str = "conda-image.json";
#[derive(Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Receipt {
    version: u32,
    tree: gripsack_store::hash::PayloadHash,
    runtime: CondaRuntimeReceipt,
}
pub(super) struct Source {
    pub artifact: Artifact,
    pub receipt: CondaRuntimeReceipt,
    root: PathBuf,
    staging: Option<tempfile::TempDir>,
}
pub(super) type Materializations = BTreeMap<String, Source>;

pub(super) fn prepare(
    ctx: &Ctx,
    session: &LifecycleSession,
    image: &ImageOutput,
    selection: &Selection<'_>,
    realized: &Realization<'_>,
) -> Result<Materializations, ExecError> {
    artifact::authority(ctx, session)?;
    let mut sources = BTreeMap::new();
    for placement in plan::placements(image, selection, realized)? {
        let Some(native) = &placement.package.conda else {
            continue;
        };
        let prefix = placement.destination.path.as_str();
        let key = conda::image_key(&native.closure, prefix);
        let root = gripsack_store::content_path(&ctx.home, NAMESPACE, &key);
        let cached: Option<Receipt> = artifact::read_receipt(ctx, &root, RECEIPT)?;
        let (mut admitted, staging) = if let Some(cached) = cached {
            let admitted = conda::inspect_image(
                placement.name,
                &ctx.home,
                &native.closure,
                prefix,
                &root.join("payload"),
            )?;
            if cached.version != 1
                || cached.tree != admitted.artifact.tree
                || cached.runtime != admitted.receipt
            {
                return Err(failure(
                    image,
                    "retained image materialization differs from its original archives or receipt",
                ));
            }
            (admitted, None)
        } else {
            let staging = tempfile::Builder::new()
                .prefix("grip-image-conda-")
                .tempdir()?;
            let object = staging.path().join("object");
            std::fs::create_dir(&object)?;
            let admitted = conda::materialize_locked(
                ctx,
                session,
                placement.name,
                &native.closure,
                CondaDestination::Image {
                    prefix,
                    staging: &object.join("payload"),
                },
            )?;
            artifact::write_receipt(
                &object,
                RECEIPT,
                &Receipt {
                    version: 1,
                    tree: admitted.artifact.tree.clone(),
                    runtime: admitted.receipt.clone(),
                },
            )?;
            (admitted, Some(staging))
        };
        admitted.artifact.retention.insert(root.clone());
        sources.insert(
            placement.name.to_owned(),
            Source {
                artifact: admitted.artifact,
                receipt: admitted.receipt,
                root,
                staging,
            },
        );
    }
    Ok(sources)
}

pub(super) fn publish(
    ctx: &Ctx,
    session: &LifecycleSession,
    sources: &Materializations,
) -> Result<(), ExecError> {
    artifact::authority(ctx, session)?;
    for source in sources.values() {
        if let Some(staging) = &source.staging {
            crate::source::publish(ctx, NAMESPACE, &staging.path().join("object"), &source.root)?;
        }
    }
    Ok(())
}
