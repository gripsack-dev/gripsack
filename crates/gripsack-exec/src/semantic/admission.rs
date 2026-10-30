use super::model::*;
use serde::{
    Deserialize, Deserializer,
    de::{self, MapAccess, SeqAccess, Visitor},
};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    fmt,
    io::{Read, Write},
    path::Path,
};

/// Reject duplicates at every JSON object depth before ordinary serde DTO decoding.
struct Unique(Value);
impl<'de> Deserialize<'de> for Unique {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct UniqueVisitor;
        impl<'de> Visitor<'de> for UniqueVisitor {
            type Value = Unique;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("unambiguous JSON")
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Unique, E> {
                serde_json::Number::from_f64(v)
                    .map(|v| Unique(Value::Number(v)))
                    .ok_or_else(|| E::custom("nonfinite number"))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Unique, E> {
                Ok(Unique(Value::Null))
            }
            fn visit_none<E: de::Error>(self) -> Result<Unique, E> {
                Ok(Unique(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Unique, A::Error> {
                let mut values = Vec::new();
                while let Some(Unique(value)) = seq.next_element()? {
                    if values.len() >= MAX_FACTS {
                        return Err(de::Error::custom("collection bound"));
                    }
                    values.push(value);
                }
                Ok(Unique(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Unique, A::Error> {
                let mut values = serde_json::Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if values.contains_key(&key) {
                        return Err(de::Error::custom("duplicate field"));
                    }
                    if values.len() >= MAX_FACTS {
                        return Err(de::Error::custom("collection bound"));
                    }
                    let Unique(value) = map.next_value()?;
                    values.insert(key, value);
                }
                Ok(Unique(Value::Object(values)))
            }
        }
        deserializer.deserialize_any(UniqueVisitor)
    }
}

pub fn parse_json(bytes: &[u8]) -> Result<Value, SnapshotError> {
    if bytes.len() > MAX_SNAPSHOT_BYTES {
        return Err(SnapshotError::Bound("bytes"));
    }
    // Lexical depth check precedes recursive decoding; brackets within strings are data.
    let (mut depth, mut quoted, mut escaped) = (0usize, false, false);
    for &byte in bytes {
        if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
        } else {
            match byte {
                b'"' => quoted = true,
                b'[' | b'{' => {
                    depth += 1;
                    if depth > MAX_DEPTH {
                        return Err(SnapshotError::Bound("depth"));
                    }
                }
                b']' | b'}' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
    }
    Ok(serde_json::from_slice::<Unique>(bytes)?.0)
}

pub fn parse_snapshot(bytes: &[u8]) -> Result<SemanticSnapshot, SnapshotError> {
    SemanticSnapshot::admit(serde_json::from_value(parse_json(bytes)?)?)
}

fn safe_text(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_TEXT_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"/_-.:%~".contains(&byte))
}
pub(super) fn admit(mut document: SnapshotDocument) -> Result<SemanticSnapshot, SnapshotError> {
    if document.version != SNAPSHOT_VERSION {
        return Err(SnapshotError::Version(document.version));
    }
    if document.canonicalization_version != CANONICALIZATION_VERSION
        || document.policy_version != POLICY_VERSION
    {
        return Err(SnapshotError::Invalid(
            "incompatible canonicalization or policy version",
        ));
    }
    if document.nodes.len() > MAX_NODES {
        return Err(SnapshotError::Bound("nodes"));
    }
    if document.edges.len() > MAX_EDGES {
        return Err(SnapshotError::Bound("edges"));
    }
    if document
        .generation
        .as_ref()
        .is_some_and(|id| id.parse::<gripsack_store::GenerationId>().is_err())
    {
        return Err(SnapshotError::Invalid("generation identity"));
    }
    let mut ids = BTreeSet::new();
    let mut reasons: BTreeSet<_> = document
        .completeness
        .unknown_reasons
        .iter()
        .copied()
        .collect();
    let mut facts = 0usize;
    let mut locations = 0usize;
    for node in &mut document.nodes {
        if !safe_text(&node.id.0) || !ids.insert(node.id.clone()) {
            return Err(SnapshotError::Invalid("duplicate or unsafe subject"));
        }
        facts += node.facts.len();
        locations += node.provenance.len();
        if facts > MAX_FACTS || locations > MAX_FACTS {
            return Err(SnapshotError::Bound("facts"));
        }
        for location in &node.provenance {
            if location.source_label.as_ref().is_some_and(|text| {
                !safe_text(text) || text.starts_with('/') || text.contains("://")
            }) {
                return Err(SnapshotError::Invalid("unsafe provenance"));
            }
        }
        let mut paths = BTreeSet::new();
        for fact in &node.facts {
            if !safe_text(&fact.path.0) || !paths.insert(&fact.path) {
                return Err(SnapshotError::Invalid("duplicate or unsafe field path"));
            }
            if let KnowledgeState::Unknown { reason } = fact.knowledge {
                reasons.insert(reason);
            }
        }
        node.facts.sort_unstable_by(|a, b| a.path.cmp(&b.path));
    }
    let mut edges = BTreeSet::new();
    for edge in &document.edges {
        if !ids.contains(&edge.dependency) || !ids.contains(&edge.consumer) || !edges.insert(edge) {
            return Err(SnapshotError::Invalid("dangling or duplicate edge"));
        }
    }
    admit_acyclic_dependencies(&document)?;
    document.nodes.sort_unstable_by(|a, b| a.id.cmp(&b.id));
    document.edges.sort_unstable();
    document.completeness.redacted |= reasons.contains(&UnknownReason::Redacted);
    document.completeness.complete &= reasons.is_empty() && !document.completeness.truncated;
    document.completeness.unknown_reasons = reasons.into_iter().collect();
    // The in-process builder has the same serialized resource budget as imports.
    encode_bounded(&document)?;
    Ok(SemanticSnapshot(document))
}

pub(super) fn encode_bounded(value: &impl serde::Serialize) -> Result<Vec<u8>, SnapshotError> {
    struct Encoder(Vec<u8>);
    impl Write for Encoder {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > MAX_SNAPSHOT_BYTES - self.0.len() {
                return Err(std::io::Error::other("semantic snapshot byte bound"));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut output = Encoder(Vec::new());
    serde_json::to_writer(&mut output, value)?;
    Ok(output.0)
}

/// Runtime/retention cycles are meaningful and comparison uses visited states.
/// Production and task prerequisite cycles do not have an admitted execution.
fn admit_acyclic_dependencies(document: &SnapshotDocument) -> Result<(), SnapshotError> {
    use std::collections::{BTreeMap, VecDeque};
    let mut incoming: BTreeMap<_, usize> =
        document.nodes.iter().map(|node| (&node.id, 0)).collect();
    let mut consumers: BTreeMap<_, Vec<_>> = BTreeMap::new();
    for edge in &document.edges {
        if matches!(
            edge.kind,
            EdgeKind::Production | EdgeKind::BuildInput | EdgeKind::TaskPrerequisite
        ) {
            *incoming.get_mut(&edge.consumer).expect("admitted consumer") += 1;
            consumers
                .entry(&edge.dependency)
                .or_default()
                .push(&edge.consumer);
        }
    }
    let mut ready: VecDeque<_> = incoming
        .iter()
        .filter_map(|(id, count)| (*count == 0).then_some(*id))
        .collect();
    let mut visited = 0;
    while let Some(id) = ready.pop_front() {
        visited += 1;
        for consumer in consumers.get(id).into_iter().flatten() {
            let count = incoming.get_mut(consumer).expect("admitted consumer");
            *count -= 1;
            if *count == 0 {
                ready.push_back(*consumer);
            }
        }
    }
    if visited != incoming.len() {
        return Err(SnapshotError::Invalid(
            "cyclic production or task prerequisites",
        ));
    }
    Ok(())
}

pub(super) fn read_bounded(path: &Path) -> Result<Vec<u8>, SnapshotError> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(SnapshotError::Invalid("not a regular file"));
    }
    let mut bytes = Vec::new();
    file.take((MAX_SNAPSHOT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_SNAPSHOT_BYTES {
        return Err(SnapshotError::Bound("bytes"));
    }
    Ok(bytes)
}
pub fn read_snapshot(path: &Path) -> Result<SemanticSnapshot, SnapshotError> {
    parse_snapshot(&read_bounded(path)?)
}

/// Explicit export only. Never follows or replaces an existing path; private from creation.
pub fn write_snapshot_private(
    path: &Path,
    snapshot: &SemanticSnapshot,
) -> Result<(), SnapshotError> {
    use std::os::unix::fs::OpenOptionsExt;
    let bytes = encode_bounded(snapshot)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}
