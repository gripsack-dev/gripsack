use crate::FetchSpec;
use crate::workspace_v6::source::LockedSource;
use serde::{
    Deserialize, Deserializer,
    de::{Error, MapAccess, Visitor},
};
use std::{
    collections::{BTreeMap, btree_map::Entry},
    fmt,
    marker::PhantomData,
};

/// JSON object keys are identities here; duplicate keys must not replace an
/// earlier platform or imported-definition pin during deserialization.
pub(super) fn unique_map<'de, D, V>(decoder: D) -> Result<BTreeMap<String, V>, D::Error>
where
    D: Deserializer<'de>,
    V: Deserialize<'de>,
{
    struct UniqueMap<V>(PhantomData<V>);
    impl<'de, V: Deserialize<'de>> Visitor<'de> for UniqueMap<V> {
        type Value = BTreeMap<String, V>;
        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("an object with unique keys")
        }
        fn visit_map<A: MapAccess<'de>>(self, mut input: A) -> Result<Self::Value, A::Error> {
            let mut output = BTreeMap::new();
            while let Some(key) = input.next_key::<String>()? {
                match output.entry(key) {
                    Entry::Vacant(entry) => {
                        entry.insert(input.next_value()?);
                    }
                    Entry::Occupied(entry) => {
                        return Err(A::Error::custom(format!(
                            "duplicate identity {:?}",
                            entry.key()
                        )));
                    }
                }
            }
            Ok(output)
        }
    }
    decoder.deserialize_map(UniqueMap(PhantomData))
}

/// Fail-closed locked-source decoder: the `fetch` variant keeps the
/// per-kind fetch field allowlist; the Conda/Pixi variants are closed
/// by their `deny_unknown_fields` records.
pub(super) fn source<'de, D: Deserializer<'de>>(decoder: D) -> Result<LockedSource, D::Error> {
    let fields: BTreeMap<String, serde_json::Value> = unique_map(decoder)?;
    let kind = fields
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| D::Error::custom("locked source is missing its kind"))?;
    match kind {
        "fetch" => {
            if fields.len() != 2 {
                return Err(D::Error::custom("unknown locked fetch-source field"));
            }
            let fetch = fields
                .get("fetch")
                .and_then(serde_json::Value::as_object)
                .ok_or_else(|| {
                    D::Error::custom("locked fetch source is missing its fetch spec")
                })?;
            let fetch_kind = fetch
                .get("kind")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| D::Error::custom("fetch is missing its kind"))?;
            let allowed = crate::tagged::allowed_fetch_fields(fetch_kind)
                .ok_or_else(|| D::Error::custom("unknown locked fetch kind"))?;
            for field in fetch.keys() {
                if !allowed.contains(&field.as_str()) {
                    return Err(D::Error::custom(format!(
                        "unknown locked fetch field {field:?}"
                    )));
                }
            }
            let spec = FetchSpec::deserialize(serde::de::value::MapDeserializer::new(
                fetch.iter().map(|(key, value)| (key.clone(), value.clone())),
            ))
            .map_err(D::Error::custom)?;
            Ok(LockedSource::Fetch { fetch: spec })
        }
        "conda_environment" | "pixi_lock" => {
            LockedSource::deserialize(serde::de::value::MapDeserializer::new(fields.into_iter()))
                .map_err(D::Error::custom)
        }
        _ => Err(D::Error::custom(format!(
            "unknown locked source kind {kind:?}"
        ))),
    }
}
