//! The new pointer intentionally lacks legacy required generation/intents
//! fields, so an older binary refuses it rather than replaying without IDs.
use super::{PendingIntent, model::ActivationId};
use crate::GenerationId;
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;

#[derive(Serialize)]
pub(super) struct PointerRecord {
    pub version: u32,
    pub instance: ActivationId,
}

pub(super) enum PendingPointer {
    Current {
        version: u32,
        instance: ActivationId,
    },
    Legacy {
        generation: GenerationId,
        intents: Vec<PendingIntent>,
    },
}

impl<'de> Deserialize<'de> for PendingPointer {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::{Error, MapAccess, Visitor};
        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "snake_case")]
        enum Field {
            Version,
            Instance,
            Generation,
            Intents,
        }
        struct PointerVisitor;
        impl<'de> Visitor<'de> for PointerVisitor {
            type Value = PendingPointer;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter
                    .write_str("a versioned activation pointer or a complete legacy pending record")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut version: Option<u32> = None;
                let mut instance: Option<ActivationId> = None;
                let mut generation: Option<u64> = None;
                let mut intents: Option<Vec<PendingIntent>> = None;
                while let Some(field) = map.next_key()? {
                    match field {
                        Field::Version => {
                            if version.is_some() {
                                return Err(A::Error::duplicate_field("version"));
                            }
                            version = Some(map.next_value()?);
                        }
                        Field::Instance => {
                            if instance.is_some() {
                                return Err(A::Error::duplicate_field("instance"));
                            }
                            instance = Some(map.next_value()?);
                        }
                        Field::Generation => {
                            if generation.is_some() {
                                return Err(A::Error::duplicate_field("generation"));
                            }
                            generation = Some(map.next_value()?);
                        }
                        Field::Intents => {
                            if intents.is_some() {
                                return Err(A::Error::duplicate_field("intents"));
                            }
                            intents = Some(map.next_value()?);
                        }
                    }
                }
                match version {
                    Some(version @ (super::LEGACY_FORMAT_VERSION | super::FORMAT_VERSION))
                        if generation.is_none() && intents.is_none() =>
                    {
                        Ok(PendingPointer::Current {
                            version,
                            instance: instance
                                .ok_or_else(|| A::Error::missing_field("instance"))?,
                        })
                    }
                    Some(super::LEGACY_FORMAT_VERSION | super::FORMAT_VERSION) => Err(
                        A::Error::custom("activation pointer mixes current and legacy fields"),
                    ),
                    Some(_) => Err(A::Error::custom("unsupported activation pointer version")),
                    None if instance.is_some() => Err(A::Error::missing_field("version")),
                    None => Ok(PendingPointer::Legacy {
                        generation: GenerationId::new(
                            generation.ok_or_else(|| A::Error::missing_field("generation"))?,
                        ),
                        intents: {
                            let intents =
                                intents.ok_or_else(|| A::Error::missing_field("intents"))?;
                            if intents.iter().any(|intent| intent.action.requires_v2()) {
                                return Err(A::Error::custom(
                                    "workspace actions require activation version 2",
                                ));
                            }
                            intents
                        },
                    }),
                }
            }
        }
        deserializer.deserialize_map(PointerVisitor)
    }
}
