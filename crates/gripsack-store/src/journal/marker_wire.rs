//! Versioned transaction markers, with an explicit read-only legacy boundary.
//! Required nullable predecessors never default to fresh-machine authority.
use super::marker::RunOp;
use crate::{
    GenerationId,
    selection_wire::{SelectionWire, serialize_optional},
};
use gripsack_policy::selection::SelectionIdentity;
use serde::{Deserialize, Deserializer, Serialize, Serializer, ser::SerializeStruct};
use std::{fmt, io};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MarkerFormat {
    LegacyGeneration,
    Transaction,
}

#[derive(Debug)]
pub(crate) struct RunMarker {
    pub(crate) previous: Option<SelectionIdentity>,
    pub(crate) target: SelectionIdentity,
    pub(crate) op: RunOp,
    format: MarkerFormat,
}

impl RunMarker {
    pub(super) fn transaction(
        previous: Option<SelectionIdentity>,
        target: SelectionIdentity,
        op: RunOp,
    ) -> io::Result<Self> {
        if target.transaction_id().is_none() || previous.as_ref() == Some(&target) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "a new transaction requires a distinct transaction-bound target",
            ));
        }
        Ok(Self {
            previous,
            target,
            op,
            format: MarkerFormat::Transaction,
        })
    }

    pub(super) fn admit_recovery(&self) -> io::Result<()> {
        if self.format == MarkerFormat::LegacyGeneration
            && self.previous.as_ref() == Some(&self.target)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "legacy same-generation journal has no transaction identity; commitment is ambiguous and the journal is retained",
            ));
        }
        Ok(())
    }
}

struct OptionalSelection<'a>(&'a Option<SelectionIdentity>);
impl Serialize for OptionalSelection<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serialize_optional(self.0, serializer)
    }
}

impl Serialize for RunMarker {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.format {
            MarkerFormat::LegacyGeneration => {
                let mut value = serializer.serialize_struct("LegacyRunMarker", 3)?;
                value.serialize_field(
                    "previous_generation",
                    &self.previous.as_ref().map(|id| id.generation().value()),
                )?;
                value.serialize_field("target_generation", &self.target.generation().value())?;
                value.serialize_field("op", &self.op)?;
                value.end()
            }
            MarkerFormat::Transaction => {
                let mut value = serializer.serialize_struct("RunMarker", 4)?;
                value.serialize_field("version", &1_u32)?;
                value.serialize_field("previous_selection", &OptionalSelection(&self.previous))?;
                value.serialize_field("target_selection", &SelectionWire(self.target))?;
                value.serialize_field("op", &self.op)?;
                value.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for RunMarker {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::{Error, IgnoredAny, MapAccess, Visitor};
        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "snake_case")]
        enum Field {
            Version,
            PreviousSelection,
            TargetSelection,
            PreviousGeneration,
            TargetGeneration,
            Op,
            #[serde(other)]
            Unknown,
        }
        struct MarkerVisitor;
        impl<'de> Visitor<'de> for MarkerVisitor {
            type Value = RunMarker;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(
                    "a versioned transaction marker or a complete legacy generation marker",
                )
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut version: Option<u32> = None;
                let mut previous_selection: Option<Option<SelectionWire>> = None;
                let mut target_selection: Option<SelectionWire> = None;
                let mut previous: Option<Option<u64>> = None;
                let mut target: Option<u64> = None;
                let mut op: Option<RunOp> = None;
                while let Some(field) = map.next_key()? {
                    match field {
                        Field::Version => {
                            if version.is_some() {
                                return Err(A::Error::duplicate_field("version"));
                            }
                            version = Some(map.next_value()?);
                        }
                        Field::PreviousSelection => {
                            if previous_selection.is_some() {
                                return Err(A::Error::duplicate_field("previous_selection"));
                            }
                            previous_selection = Some(map.next_value()?);
                        }
                        Field::TargetSelection => {
                            if target_selection.is_some() {
                                return Err(A::Error::duplicate_field("target_selection"));
                            }
                            target_selection = Some(map.next_value()?);
                        }
                        Field::PreviousGeneration => {
                            if previous.is_some() {
                                return Err(A::Error::duplicate_field("previous_generation"));
                            }
                            previous = Some(map.next_value()?);
                        }
                        Field::TargetGeneration => {
                            if target.is_some() {
                                return Err(A::Error::duplicate_field("target_generation"));
                            }
                            target = Some(map.next_value()?);
                        }
                        Field::Op => {
                            if op.is_some() {
                                return Err(A::Error::duplicate_field("op"));
                            }
                            op = Some(map.next_value()?);
                        }
                        Field::Unknown => {
                            let _ = map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                let op = op.ok_or_else(|| A::Error::missing_field("op"))?;
                match version {
                    Some(1) => {
                        if previous.is_some() || target.is_some() {
                            return Err(A::Error::custom(
                                "transaction marker mixes legacy generation fields",
                            ));
                        }
                        RunMarker::transaction(
                            previous_selection
                                .ok_or_else(|| A::Error::missing_field("previous_selection"))?
                                .map(|value| value.0),
                            target_selection
                                .ok_or_else(|| A::Error::missing_field("target_selection"))?
                                .0,
                            op,
                        )
                        .map_err(A::Error::custom)
                    }
                    Some(_) => Err(A::Error::custom("unsupported transaction marker version")),
                    None => {
                        if previous_selection.is_some() || target_selection.is_some() {
                            return Err(A::Error::missing_field("version"));
                        }
                        Ok(RunMarker {
                            previous: previous
                                .ok_or_else(|| A::Error::missing_field("previous_generation"))?
                                .map(|number| SelectionIdentity::legacy(GenerationId::new(number))),
                            target: SelectionIdentity::legacy(GenerationId::new(
                                target
                                    .ok_or_else(|| A::Error::missing_field("target_generation"))?,
                            )),
                            op,
                            format: MarkerFormat::LegacyGeneration,
                        })
                    }
                }
            }
        }
        deserializer.deserialize_map(MarkerVisitor)
    }
}
