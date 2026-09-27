//! Explicit selection/transaction wire seam. Policy keeps compact identities;
//! this module alone owns their canonical JSON and filesystem text encoding.
use crate::GenerationId;
use gripsack_policy::selection::{SelectionIdentity, TransactionId};
use serde::{Deserialize, Deserializer, Serialize, Serializer, ser::SerializeStruct};
use std::{fmt, io};

pub(crate) struct TransactionText<'a>(pub(crate) &'a TransactionId);

impl fmt::Display for TransactionText<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0.as_bytes() {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl Serialize for TransactionText<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

pub(crate) fn parse_transaction(text: &str) -> io::Result<TransactionId> {
    fn digit(byte: u8) -> Option<u8> {
        match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            _ => None,
        }
    }
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "transaction identity must be 64 lowercase hexadecimal characters",
        )
    };
    if text.len() != 64 {
        return Err(invalid());
    }
    let mut bytes = [0; 32];
    for (target, pair) in bytes.iter_mut().zip(text.as_bytes().as_chunks::<2>().0) {
        *target =
            (digit(pair[0]).ok_or_else(invalid)? << 4) | digit(pair[1]).ok_or_else(invalid)?;
    }
    Ok(TransactionId::from_bytes(bytes))
}

struct TransactionWire(TransactionId);

impl<'de> Deserialize<'de> for TransactionWire {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct IdentityVisitor;
        impl serde::de::Visitor<'_> for IdentityVisitor {
            type Value = TransactionWire;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a canonical transaction identity")
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                parse_transaction(value)
                    .map(TransactionWire)
                    .map_err(E::custom)
            }
        }
        deserializer.deserialize_str(IdentityVisitor)
    }
}

pub(crate) struct SelectionWire(pub(crate) SelectionIdentity);

impl Serialize for SelectionWire {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serialize(&self.0, serializer)
    }
}

impl<'de> Deserialize<'de> for SelectionWire {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
        enum Fields {
            Legacy {
                #[serde(with = "crate::generation_wire")]
                generation: GenerationId,
            },
            Transaction {
                #[serde(with = "crate::generation_wire")]
                generation: GenerationId,
                transaction: TransactionWire,
            },
        }
        Ok(Self(match Fields::deserialize(deserializer)? {
            Fields::Legacy { generation } => SelectionIdentity::legacy(generation),
            Fields::Transaction {
                generation,
                transaction,
            } => SelectionIdentity::transaction(generation, transaction.0),
        }))
    }
}

pub(crate) fn serialize<S: Serializer>(
    selection: &SelectionIdentity,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let transaction = selection.transaction_id();
    let mut value = serializer.serialize_struct(
        "SelectionIdentity",
        if transaction.is_some() { 3 } else { 2 },
    )?;
    value.serialize_field(
        "kind",
        if transaction.is_some() {
            "transaction"
        } else {
            "legacy"
        },
    )?;
    value.serialize_field("generation", &selection.generation().value())?;
    if let Some(transaction) = transaction {
        value.serialize_field("transaction", &TransactionText(transaction))?;
    }
    value.end()
}

pub(crate) fn serialize_optional<S: Serializer>(
    selection: &Option<SelectionIdentity>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match selection {
        Some(selection) => serializer.serialize_some(&SelectionReference(selection)),
        None => serializer.serialize_none(),
    }
}

struct SelectionReference<'a>(&'a SelectionIdentity);
impl Serialize for SelectionReference<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serialize(self.0, serializer)
    }
}
