//! Managed-block upsert planning without acquiring the hosting file. The
//! kernel decides (plan/0056 K1 for blocks) and splices (K5).
use super::*;
use gripsack_kernel::block::{BlockDecision, BlockIntent, BlockRefusal};

pub(super) fn plan_merge(
    view: &DestView,
    payload: &str,
    permissions: WritePermissions,
    block_id: Option<&store::ManagedBlockId>,
    source: BlockSource,
) -> Result<Op, ExecError> {
    let fail = |detail: String| ExecError::Step {
        output: view.output.to_string(),
        step: match source {
            BlockSource::Declared => "deploy",
            BlockSource::Retained { .. } => "rollback",
        }
        .into(),
        detail: format!("{}: {detail}", view.entry.to),
    };
    let host = crate::managed_blocks::host(view.observed.as_ref());
    let existing = match &view.observed {
        Some(crate::deploy::Observation::File { bytes, .. }) => std::str::from_utf8(bytes)
            .map_err(|e| fail(format!("hosting file is not UTF-8: {e}")))?,
        _ => "",
    };
    let owner = block_id.map_or(view.output, store::ManagedBlockId::as_str);
    let blocks = crate::managed_blocks::ManagedBlockSet::parse(existing, owner)
        .map_err(|e| fail(e.to_string()))?;
    let mode = permissions.resolve(view);
    let hash = crate::managed_blocks::content_hash(payload);
    let desired = store::hash::kernel::digest_hex(hash.as_str()).expect("sha256 hex");
    let intent = match source {
        BlockSource::Declared => BlockIntent::Deploy { desired, mode },
        BlockSource::Retained { recorded } => BlockIntent::Restore {
            retained: desired,
            recorded: store::hash::kernel::digest_hex(recorded.as_str()),
            mode,
        },
    };
    let record = view.prev_entry().map(crate::managed_blocks::record);
    match blocks.decide(record, &host, &intent) {
        BlockDecision::Refuse(BlockRefusal::Foreign) => Err(fail(
            "hosting file is a symlink; refusing to replace foreign ownership".into(),
        )),
        BlockDecision::Refuse(BlockRefusal::Unrestorable) => Err(fail(format!(
            "the retained block payload ({hash}) is not the recorded block; cannot restore it \
             exactly"
        ))),
        // mode evidence conflicts with the live file: an explicitly
        // non-authoritative observation of the whole hosting text
        BlockDecision::Preserve => {
            let Some(crate::deploy::Observation::File {
                mode: live_mode, ..
            }) = &view.observed
            else {
                return Err(fail("planner bug: preserved a non-file".into()));
            };
            Ok(view
                .base(
                    OpKind::Preserved,
                    None,
                    Intended::Object(view.observed_identity().expect("block implies live")),
                )
                .with_produces(view.produces(
                    store::canonical_bytes_hash(existing.as_bytes()).into(),
                    Some(*live_mode),
                    true,
                )))
        }
        BlockDecision::Satisfied => Ok(view
            .base(
                OpKind::Satisfied,
                None,
                Intended::Object(view.observed_identity().expect("satisfied implies live")),
            )
            .with_produces(view.produces(hash.into(), Some(mode), false))),
        BlockDecision::Write => {
            let spliced = blocks
                .upsert(
                    owner,
                    &view.dest,
                    view.entry.marker.as_deref(),
                    payload,
                    mode,
                )
                .map_err(|e| fail(e.to_string()))?;
            Ok(view
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
                .with_note(blocks.report_note()))
        }
        BlockDecision::Remove | BlockDecision::Keep | BlockDecision::Absent => {
            Err(fail("planner bug: a block deploy decided a removal".into()))
        }
    }
}
