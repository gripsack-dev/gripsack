//! Managed-block upsert planning without acquiring the hosting file.
use super::*;

pub(super) fn plan_merge(
    view: &DestView,
    payload: &str,
    permissions: WritePermissions,
    block_id: Option<&store::ManagedBlockId>,
) -> Result<Op, ExecError> {
    let fail = |detail: String| ExecError::Step {
        output: view.output.to_string(),
        step: "deploy".into(),
        detail: format!("{}: {detail}", view.entry.to),
    };
    let existing = match &view.observed {
        None => "",
        Some(crate::deploy::Observation::File { bytes, .. }) => std::str::from_utf8(bytes)
            .map_err(|e| fail(format!("hosting file is not UTF-8: {e}")))?,
        Some(crate::deploy::Observation::Symlink { .. }) => {
            return Err(fail(
                "hosting file is a symlink; refusing to replace foreign ownership".into(),
            ));
        }
    };
    let owner = block_id.map_or(view.output, store::ManagedBlockId::as_str);
    let blocks = crate::managed_blocks::ManagedBlockSet::parse(existing, owner)
        .map_err(|e| fail(e.to_string()))?;
    let mode = permissions.resolve(view);
    if let Some(crate::deploy::Observation::File {
        mode: live_mode, ..
    }) = &view.observed
        && !blocks.is_empty()
        && blocks.mode_conflicts(*live_mode, view.prev)
    {
        // An explicitly non-authoritative observation of the whole hosting text.
        return Ok(view
            .base(
                OpKind::Preserved,
                None,
                Intended::Object(view.observed_identity().expect("block implies live")),
            )
            .with_produces(view.produces(
                store::canonical_bytes_hash(existing.as_bytes()).into(),
                Some(*live_mode),
                true,
            )));
    }
    let hash = crate::managed_blocks::content_hash(payload);
    if blocks.satisfied(&hash, mode)
        && matches!(&view.observed, Some(crate::deploy::Observation::File { mode: live, .. }) if *live == mode)
    {
        return Ok(view
            .base(
                OpKind::Satisfied,
                None,
                Intended::Object(view.observed_identity().expect("satisfied implies live")),
            )
            .with_produces(view.produces(hash.into(), Some(mode), false)));
    }
    let spliced = blocks
        .upsert(
            owner,
            &view.dest,
            view.entry.marker.as_deref(),
            payload,
            mode,
        )
        .map_err(|e| fail(e.to_string()))?;
    let op = view
        .base(
            OpKind::MergeUpsert {
                block_id: block_id.cloned(),
                payload: payload.as_bytes().to_vec(),
                marker: view.entry.marker.clone(),
                mode,
            },
            Some(Authority::Update),
            Intended::Object(ObjectIdentity::File(store::canonical_bytes_identity(
                spliced.as_bytes(),
                mode,
            ))),
        )
        .with_produces(view.produces(hash.into(), Some(mode), false))
        .with_note(blocks.report_note());
    Ok(op)
}
