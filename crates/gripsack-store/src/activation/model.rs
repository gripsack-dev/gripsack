//! One persisted effective ordering. Cache coalescing happens once, before
//! publication; replay never reconstructs IDs from today's repository.
use super::PendingIntent;
use crate::selection_wire::{TransactionText, parse_transaction};
use gripsack_ir::{Action, Trigger};
use gripsack_policy::selection::{SelectionIdentity, TransactionId};
use gripsack_process::Sha256Digest;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::{fmt, io};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActivationId(pub(super) TransactionId);

impl fmt::Display for ActivationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        TransactionText(&self.0).fmt(formatter)
    }
}
impl Serialize for ActivationId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}
impl<'de> Deserialize<'de> for ActivationId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl serde::de::Visitor<'_> for Visitor {
            type Value = ActivationId;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a canonical activation instance identity")
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                parse_transaction(value)
                    .map(ActivationId)
                    .map_err(E::custom)
            }
        }
        deserializer.deserialize_str(Visitor)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct IntentId(Sha256Digest);
impl IntentId {
    pub fn digest(self) -> Sha256Digest {
        self.0
    }
}
impl fmt::Display for IntentId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct IntentOrdinal(pub(super) u32);
impl IntentOrdinal {
    pub(super) fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contributor {
    pub module: String,
    pub trigger: Trigger,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectiveIntent {
    pub(super) id: IntentId,
    pub(super) ordinal: IntentOrdinal,
    pub(super) action_digest: Sha256Digest,
    #[serde(with = "super::action_wire")]
    pub(super) action: Action,
    pub(super) contributors: Vec<Contributor>,
}
impl EffectiveIntent {
    pub fn id(&self) -> IntentId {
        self.id
    }
    pub fn ordinal(&self) -> IntentOrdinal {
        self.ordinal
    }
    pub fn action(&self) -> &Action {
        &self.action
    }
    pub fn contributors(&self) -> &[Contributor] {
        &self.contributors
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityOrigin {
    Transaction,
    LegacyIdentityUnavailable,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Plan {
    pub version: u32,
    pub instance: ActivationId,
    #[serde(with = "selection")]
    pub selection: SelectionIdentity,
    pub identity_origin: IdentityOrigin,
    pub intents: Vec<EffectiveIntent>,
}

mod selection {
    use super::*;
    pub fn serialize<S: Serializer>(
        value: &SelectionIdentity,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        crate::selection_wire::serialize(value, serializer)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<SelectionIdentity, D::Error> {
        crate::selection_wire::SelectionWire::deserialize(deserializer).map(|value| value.0)
    }
}

struct HashWriter(Sha256);
impl io::Write for HashWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
pub(crate) fn digest(value: &impl Serialize) -> io::Result<Sha256Digest> {
    let mut writer = HashWriter(Sha256::new());
    serde_json::to_writer(&mut writer, value).map_err(io::Error::other)?;
    Ok(Sha256Digest::from_bytes(writer.0.finalize().into()))
}
fn identity(
    instance: ActivationId,
    ordinal: IntentOrdinal,
    action: Sha256Digest,
    contributors: &[Contributor],
) -> io::Result<IntentId> {
    digest(&(
        "gripsack.activation.intent.v1",
        instance,
        ordinal,
        action,
        contributors,
    ))
    .map(IntentId)
}

struct Draft {
    action: Action,
    contributors: Vec<Contributor>,
}

pub(super) fn effective(
    instance: ActivationId,
    declarations: Vec<PendingIntent>,
) -> io::Result<Vec<EffectiveIntent>> {
    let mut fonts: Option<Draft> = None;
    let mut desktop: Option<Draft> = None;
    let mut rest = Vec::new();
    for declaration in declarations {
        let contributor = Contributor {
            module: declaration.module,
            trigger: declaration.trigger,
        };
        match declaration.action {
            Action::Fonts => fonts
                .get_or_insert_with(|| Draft {
                    action: Action::Fonts,
                    contributors: Vec::new(),
                })
                .contributors
                .push(contributor),
            Action::DesktopEntry => desktop
                .get_or_insert_with(|| Draft {
                    action: Action::DesktopEntry,
                    contributors: Vec::new(),
                })
                .contributors
                .push(contributor),
            action => rest.push(Draft {
                action,
                contributors: vec![contributor],
            }),
        }
    }
    fonts
        .into_iter()
        .chain(desktop)
        .chain(rest)
        .enumerate()
        .map(|(index, draft)| {
            let ordinal = IntentOrdinal(u32::try_from(index).map_err(io::Error::other)?);
            let action_digest = digest(&draft.action)?;
            Ok(EffectiveIntent {
                id: identity(instance, ordinal, action_digest, &draft.contributors)?,
                ordinal,
                action_digest,
                action: draft.action,
                contributors: draft.contributors,
            })
        })
        .collect()
}

impl Plan {
    pub(super) fn admit(&self, instance: ActivationId) -> io::Result<()> {
        let invalid = || {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "activation plan identity or effective ordering is corrupt",
            )
        };
        if self.version != 1 || self.instance != instance {
            return Err(invalid());
        }
        if self.identity_origin == IdentityOrigin::Transaction
            && self.selection.transaction_id() != Some(&instance.0)
        {
            return Err(invalid());
        }
        let mut fonts = false;
        let mut desktop = false;
        let mut ordinary = false;
        for (index, intent) in self.intents.iter().enumerate() {
            if intent.ordinal.index() != index
                || intent.contributors.is_empty()
                || intent
                    .contributors
                    .iter()
                    .any(|source| source.module.is_empty())
                || digest(&intent.action)? != intent.action_digest
                || identity(
                    instance,
                    intent.ordinal,
                    intent.action_digest,
                    &intent.contributors,
                )? != intent.id
            {
                return Err(invalid());
            }
            match intent.action {
                Action::Fonts if !fonts && !desktop && !ordinary => fonts = true,
                Action::DesktopEntry if !desktop && !ordinary => desktop = true,
                Action::Fonts | Action::DesktopEntry => return Err(invalid()),
                _ if intent.contributors.len() == 1 => ordinary = true,
                _ => return Err(invalid()),
            }
        }
        Ok(())
    }
}
