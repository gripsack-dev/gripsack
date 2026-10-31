//! Read-only explanations of the shared planner's operations. This adapter never
//! chooses or removes an operation and never grants deployment authority.
use super::config_view::{
    ConfigFormat, ConfigScope, ConfigView, ConfigViewOptions, MAX_INPUT_BYTES, compare_config,
};
use crate::ops::{Authority, ContentSource, Op, OpKind};
use gripsack_store::journal::ObjectIdentity;
use serde::Serialize;
use std::{
    fs::OpenOptions,
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    Link,
    Write,
    ManagedBlock,
    Remove,
    Satisfied,
    Deferred,
    Preserved,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationAuthority {
    Fresh,
    Update,
    TakeOver,
    Foreign,
    None,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentKnowledge {
    CapturedBytes,
    DeferredAcquisition,
    NotApplicable,
    ManagedBlockWithForeignBytes,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigurationUnknown {
    ObservationChanged,
    ObservationUnavailable,
    ObservationOversized,
    ManagedBlockBoundary,
    DeferredContent,
}
#[derive(Debug, Serialize)]
pub struct OperationExplanation {
    pub output: String,
    pub destination: String,
    pub kind: OperationKind,
    pub authority: OperationAuthority,
    pub ownership: gripsack_store::Ownership,
    pub intended_mode: Option<u32>,
    pub observed_mode: Option<u32>,
    pub content: ContentKnowledge,
    pub configuration_scope: Option<ConfigScope>,
    pub configuration: Option<ConfigView>,
    pub configuration_unknown: Option<ConfigurationUnknown>,
    pub managed_blocks_observed: Option<usize>,
    pub foreign_bytes_observed: Option<usize>,
    /// No previous foreign-content evidence is inferred from an owned block.
    pub foreign_drift_known: bool,
    pub redacted: bool,
}

/// Every returned row corresponds one-to-one and in order with the actual ops.
/// Exact configuration bytes stay private; only the config adapter's default
/// disclosure-filtered field changes may be serialized.
pub fn explain(operations: &[Op]) -> Result<Vec<OperationExplanation>, super::SnapshotError> {
    if operations.len() > super::model::MAX_NODES {
        return Err(super::SnapshotError::Bound("operation explanations"));
    }
    Ok(operations.iter().map(explain_one).collect())
}

fn explain_one(op: &Op) -> OperationExplanation {
    let (kind, content, intended_mode) = match op.kind() {
        OpKind::Link { .. } => (OperationKind::Link, ContentKnowledge::NotApplicable, None),
        OpKind::Write {
            content: ContentSource::Bytes(_),
            mode,
        } => (
            OperationKind::Write,
            ContentKnowledge::CapturedBytes,
            Some(*mode),
        ),
        OpKind::Write {
            content: ContentSource::DeferredFetch,
            mode,
        } => (
            OperationKind::Write,
            ContentKnowledge::DeferredAcquisition,
            Some(*mode),
        ),
        OpKind::MergeUpsert { mode, .. } => (
            OperationKind::ManagedBlock,
            ContentKnowledge::ManagedBlockWithForeignBytes,
            Some(*mode),
        ),
        OpKind::Remove(_) => (OperationKind::Remove, ContentKnowledge::NotApplicable, None),
        OpKind::Satisfied => (
            OperationKind::Satisfied,
            ContentKnowledge::NotApplicable,
            None,
        ),
        OpKind::Deferred => (
            OperationKind::Deferred,
            ContentKnowledge::DeferredAcquisition,
            None,
        ),
        OpKind::Preserved => (
            OperationKind::Preserved,
            ContentKnowledge::NotApplicable,
            None,
        ),
    };
    let mut row = OperationExplanation {
        output: label(op.output()),
        destination: label(op.declared_to()),
        kind,
        authority: match op.authority() {
            Some(Authority::Fresh) => OperationAuthority::Fresh,
            Some(Authority::Update) => OperationAuthority::Update,
            Some(Authority::TakeOver) => OperationAuthority::TakeOver,
            Some(Authority::Foreign) => OperationAuthority::Foreign,
            None => OperationAuthority::None,
        },
        ownership: op.mode().clone(),
        intended_mode,
        observed_mode: None,
        content,
        configuration_scope: None,
        configuration: None,
        configuration_unknown: None,
        managed_blocks_observed: None,
        foreign_bytes_observed: None,
        foreign_drift_known: false,
        redacted: true,
    };
    match op.kind() {
        OpKind::Write {
            content: ContentSource::Bytes(after),
            ..
        } => {
            row.configuration_scope = Some(ConfigScope::WholeFile);
            match before_bytes(op) {
                Ok((before, mode)) => {
                    row.observed_mode = mode;
                    let format = configuration_format(op);
                    row.configuration = Some(compare_config(
                        &before,
                        after,
                        format,
                        ConfigScope::WholeFile,
                        &ConfigViewOptions::default(),
                    ));
                }
                Err(reason) => row.configuration_unknown = Some(reason),
            }
        }
        OpKind::MergeUpsert {
            block_id,
            payload,
            marker,
            mode,
        } => {
            row.configuration_scope = Some(ConfigScope::ManagedBlock);
            if let Err(reason) = explain_block(
                op,
                block_id.as_ref(),
                payload,
                marker.as_deref(),
                *mode,
                &mut row,
            ) {
                row.configuration_unknown = Some(reason);
            }
        }
        OpKind::Write {
            content: ContentSource::DeferredFetch,
            ..
        }
        | OpKind::Deferred => {
            row.configuration_unknown = Some(ConfigurationUnknown::DeferredContent);
        }
        _ => {}
    }
    row
}

fn configuration_format(op: &Op) -> ConfigFormat {
    match op.dest().extension().and_then(|value| value.to_str()) {
        Some("json") => ConfigFormat::Json,
        Some("toml") => ConfigFormat::Toml,
        _ => ConfigFormat::Unsupported,
    }
}

fn explain_block(
    op: &Op,
    block_id: Option<&gripsack_store::ManagedBlockId>,
    payload: &[u8],
    marker: Option<&str>,
    mode: u32,
    row: &mut OperationExplanation,
) -> Result<(), ConfigurationUnknown> {
    use crate::managed_blocks::ManagedBlockSet;
    if payload.len() > MAX_INPUT_BYTES {
        return Err(ConfigurationUnknown::ObservationOversized);
    }
    let (before, observed_mode) = before_bytes(op)?;
    row.observed_mode = observed_mode;
    let before =
        std::str::from_utf8(&before).map_err(|_| ConfigurationUnknown::ManagedBlockBoundary)?;
    let owner = block_id.map_or(op.output(), gripsack_store::ManagedBlockId::as_str);
    let blocks = ManagedBlockSet::parse(before, owner)
        .map_err(|_| ConfigurationUnknown::ManagedBlockBoundary)?;
    row.managed_blocks_observed = Some(blocks.blocks().len());
    row.foreign_bytes_observed = Some(
        before.len()
            - blocks
                .blocks()
                .iter()
                .map(|block| block.range.len())
                .sum::<usize>(),
    );
    if blocks.blocks().len() > 1 {
        return Err(ConfigurationUnknown::ManagedBlockBoundary);
    }
    let payload =
        std::str::from_utf8(payload).map_err(|_| ConfigurationUnknown::ManagedBlockBoundary)?;
    let after = blocks
        .upsert(owner, op.dest(), marker, payload, mode)
        .map_err(|_| ConfigurationUnknown::ManagedBlockBoundary)?;
    let generated = ManagedBlockSet::parse(&after, owner)
        .map_err(|_| ConfigurationUnknown::ManagedBlockBoundary)?;
    let after = generated
        .blocks()
        .first()
        .ok_or(ConfigurationUnknown::ManagedBlockBoundary)?;
    let before = blocks.blocks().first().map_or("", |block| block.content);
    row.configuration = Some(compare_config(
        before.as_bytes(),
        after.content.as_bytes(),
        configuration_format(op),
        ConfigScope::ManagedBlock,
        &ConfigViewOptions::default(),
    ));
    Ok(())
}

fn before_bytes(op: &Op) -> Result<(Vec<u8>, Option<u32>), ConfigurationUnknown> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(op.dest());
    let file = match file {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && op.observed().is_none() => {
            return Ok((Vec::new(), None));
        }
        Err(_) => return Err(ConfigurationUnknown::ObservationUnavailable),
    };
    let metadata = file
        .metadata()
        .map_err(|_| ConfigurationUnknown::ObservationUnavailable)?;
    if !metadata.is_file() {
        return Err(ConfigurationUnknown::ObservationUnavailable);
    }
    if metadata.len() > MAX_INPUT_BYTES as u64 {
        return Err(ConfigurationUnknown::ObservationOversized);
    }
    let mode = metadata.mode() & 0o7777;
    let mut bytes = Vec::new();
    file.take(MAX_INPUT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ConfigurationUnknown::ObservationUnavailable)?;
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(ConfigurationUnknown::ObservationOversized);
    }
    let observed = ObjectIdentity::File(gripsack_store::canonical_bytes_identity(&bytes, mode));
    if op.observed() != Some(&observed) {
        return Err(ConfigurationUnknown::ObservationChanged);
    }
    Ok((bytes, Some(mode)))
}

fn label(value: &str) -> String {
    // Locations containing URL syntax are not useful destination spellings and
    // may contain credentials. Neither truncate them into a misleading path nor
    // print a bearer token as an operation name.
    if value.contains("://") {
        return "[redacted location]".into();
    }
    let mut result = String::new();
    for character in value.chars().take(super::model::MAX_TEXT_BYTES) {
        if character.is_control() {
            result.extend(character.escape_default());
        } else {
            result.push(character);
        }
    }
    if value.chars().count() > super::model::MAX_TEXT_BYTES {
        result.push_str("[truncated]");
    }
    result
}
