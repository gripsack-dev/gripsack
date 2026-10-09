//! Image consumers reuse frozen producers; Conda archives are independently
//! materialized for image prefixes. BuildKit composes, native storage retains.
mod contents;
mod materialize;
mod plan;
mod runtime;

use super::{
    artifact,
    realize::{BuildOptions, BuiltOutput, Realization},
    roots::{self, RetentionSet, RootId},
    selection::Selection,
    solve,
};
use crate::{Ctx, ExecError, LifecycleSession};
use gripsack_buildkit::{
    oci::{self, BlobDigest, OciLimits},
    plan::ExporterPlan,
};
use gripsack_ir::{Diagnostic, codes, workspace_model::ImageOutput};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

const RECEIPT: &str = "image.json";
const ARCHIVE: &str = "image.oci.tar";
#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImageReceipt {
    version: u32,
    identity: plan::ImageIdentity,
    archive_tree: gripsack_store::hash::PayloadHash,
    manifest: BlobDigest,
    external_runtime_requirements: Vec<String>,
}

pub(super) fn export(
    ctx: &Ctx,
    options: &BuildOptions<'_>,
    image: &ImageOutput,
    selection: &Selection<'_>,
    realized: &Realization<'_>,
    in_flight: &BTreeSet<PathBuf>,
    session: LifecycleSession,
) -> Result<(LifecycleSession, BuiltOutput), ExecError> {
    artifact::authority(ctx, &session)?;
    let materialized = materialize::prepare(ctx, &session, image, selection, realized)?;
    let mut projection = plan::compile(image, selection, realized, &materialized)?;
    let root = gripsack_store::content_path(
        &ctx.home,
        "workspace-image",
        &projection.identity.to_string(),
    );
    let payload = root.join("payload");
    let mut paths = BTreeSet::from([root.clone()]);
    for placement in &projection.placements {
        placement.package.retain_into(&mut paths);
        paths.extend(placement.artifact.retention.iter().cloned());
    }
    let consumer = RetentionSet::admit(&session, paths)?;
    let cached: Option<ImageReceipt> = artifact::read_receipt(ctx, &root, RECEIPT)?;
    let mut exported = None;
    let session = if let Some(receipt) = cached {
        if receipt.version != 2 || receipt.identity != projection.identity {
            return Err(failure(
                image,
                "retained image receipt has a different version or identity",
            ));
        }
        let parent =
            gripsack_fs::open_dir_nofollow(ctx.home_dir()?, Path::new(gripsack_store::STORE_DIR))?;
        let object = gripsack_fs::open_dir_nofollow(
            &parent,
            Path::new(
                root.file_name()
                    .ok_or_else(|| failure(image, "image root lacks a name"))?,
            ),
        )?;
        let directory = gripsack_fs::open_dir_nofollow(&object, Path::new("payload"))?;
        let actual = inspect(ctx, image, &projection, &directory, &payload)?;
        if actual != receipt {
            return Err(failure(
                image,
                "retained image bytes or descriptors changed",
            ));
        }
        session
    } else {
        let protection = RetentionSet::admit(
            &session,
            in_flight
                .iter()
                .cloned()
                .chain(consumer.paths().iter().cloned()),
        )?;
        let (session, completed) = solve::execute(
            ctx,
            options,
            &projection.plan,
            &projection.sources,
            &mut projection.origins,
            &protection,
            session,
        )?;
        let publication = (|| -> Result<(), ExecError> {
            let directory = gripsack_fs::open(completed.output())?;
            let receipt = inspect(ctx, image, &projection, &directory, completed.output())?;
            let staging = completed
                .output()
                .parent()
                .ok_or_else(|| failure(image, "completed image export has no staging parent"))?;
            let stage = gripsack_fs::open(staging)?;
            stage.create_dir("object")?;
            let object = gripsack_fs::open_dir_nofollow(&stage, Path::new("object"))?;
            object.create_dir("payload")?;
            let destination = gripsack_fs::open_dir_nofollow(&object, Path::new("payload"))?;
            gripsack_fs::rename(
                &directory,
                Path::new(ARCHIVE),
                &destination,
                Path::new(ARCHIVE),
            )?;
            let object_path = staging.join("object");
            artifact::write_receipt(&object_path, RECEIPT, &receipt)?;
            crate::source::publish(ctx, "workspace-image", &object_path, &root)
        })();
        if let Err(error) = publication {
            completed.finish(&session)?;
            return Err(error);
        }
        exported = Some(completed);
        session
    };
    let publication = (|| -> Result<(), ExecError> {
        materialize::publish(ctx, &session, &materialized)?;
        let id = RootId::from_identity(&serde_json::to_vec(&(
            ctx.repository.identity(),
            &image.name,
        ))?);
        roots::register_output_root(&session, &id, &consumer)
    })();
    if let Some(completed) = exported {
        completed.finish(&session)?;
    }
    publication?;
    Ok((
        session,
        BuiltOutput {
            name: image.name.clone(),
            kind: "image",
            path: payload.join(ARCHIVE),
            commands: BTreeMap::new(),
        },
    ))
}

fn inspect(
    ctx: &Ctx,
    image: &ImageOutput,
    projection: &plan::ImagePlan<'_, '_>,
    directory: &gripsack_fs::Dir,
    path: &Path,
) -> Result<ImageReceipt, ExecError> {
    let mut entries = directory.entries()?;
    let entry = entries
        .next()
        .transpose()?
        .ok_or_else(|| failure(image, "OCI export is empty"))?;
    if entry.file_name() != ARCHIVE
        || !entry.file_type()?.is_file()
        || entries.next().transpose()?.is_some()
    {
        return Err(failure(
            image,
            "OCI export must contain exactly its regular archive, with no extra payload",
        ));
    }
    let ExporterPlan::Oci { config } = &projection.plan.plan().exporter else {
        unreachable!("image projection selects OCI");
    };
    let mut archive = gripsack_fs::open_file_nofollow(directory, Path::new(ARCHIVE))?.into_std();
    let limits = ctx.fetch.limits();
    let limits = OciLimits {
        archive_bytes: limits.expanded_bytes.get(),
        expanded_bytes: limits.expanded_bytes.get(),
        entries: limits.archive_entries.get(),
        metadata_bytes: limits.decoder_bytes.get(),
    };
    let checked = oci::validate(
        &mut archive,
        projection.plan.plan().platform,
        config,
        limits,
    )
    .map_err(|error| failure(image, error))?;
    contents::validate(image, &projection.placements, &checked)?;
    let external_runtime_requirements = runtime::validate(
        image,
        &projection.placements,
        &checked,
        &mut archive,
        limits,
    )?;
    for requirement in &external_runtime_requirements {
        tracing::warn!(image = %image.name, %requirement, "image requires external runtime capability; image creation does not establish the runtime kernel, GPU or CPU");
    }
    Ok(ImageReceipt {
        version: 2,
        identity: projection.identity,
        archive_tree: gripsack_store::canonical_tree_hash(path)?,
        manifest: checked.manifest(),
        external_runtime_requirements,
    })
}

fn failure(image: &ImageOutput, detail: impl std::fmt::Display) -> ExecError {
    ExecError::Gate(
        Diagnostic::error(codes::EXEC_STEP, detail.to_string())
            .with_label(Some(image.span.clone()), "image declared here"),
    )
}
