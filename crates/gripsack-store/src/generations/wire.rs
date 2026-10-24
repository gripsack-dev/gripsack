//! Manifest versions are authority boundaries: v1/v2 cannot acquire v3
//! migration recovery roots or inode-bound directory transition receipts.
use super::{Generation, GenerationId, MigrationReceipt, ModuleState};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const WORKSPACE_MANIFEST_VERSION: u32 = 2;
const MIGRATION_MANIFEST_VERSION: u32 = 3;

impl Serialize for Generation {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Manifest<'a> {
            #[serde(with = "crate::generation_wire")]
            number: GenerationId,
            modules: &'a BTreeMap<String, ModuleState>,
            #[serde(skip_serializing_if = "Vec::is_empty")]
            directory_transitions: &'a Vec<crate::journal::directory::DirectoryTransition>,
            #[serde(skip_serializing_if = "Option::is_none")]
            migration: &'a Option<MigrationReceipt>,
        }
        #[derive(Serialize)]
        struct Versioned<'a> {
            version: u32,
            generation: Manifest<'a>,
        }
        let manifest = Manifest {
            number: self.number,
            modules: &self.modules,
            directory_transitions: &self.directory_transitions,
            migration: &self.migration,
        };
        if self.migration.is_some()
            || !self.directory_transitions.is_empty()
            || self
                .modules
                .values()
                .flat_map(|state| &state.intents)
                .any(|intent| intent.action.requires_v2())
        {
            Versioned {
                version: if self.directory_transitions.is_empty() && self.migration.is_none() {
                    WORKSPACE_MANIFEST_VERSION
                } else {
                    MIGRATION_MANIFEST_VERSION
                },
                generation: manifest,
            }
            .serialize(serializer)
        } else {
            manifest.serialize(serializer)
        }
    }
}

impl<'de> Deserialize<'de> for Generation {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Manifest {
            #[serde(with = "crate::generation_wire")]
            number: GenerationId,
            modules: BTreeMap<String, ModuleState>,
            #[serde(default)]
            directory_transitions: Vec<crate::journal::directory::DirectoryTransition>,
            #[serde(default)]
            migration: Option<MigrationReceipt>,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Versioned {
            version: u32,
            generation: Manifest,
        }
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Wire {
            Versioned(Versioned),
            Legacy(Manifest),
        }
        let (manifest, version) = match Wire::deserialize(deserializer)? {
            Wire::Versioned(value)
                if matches!(
                    value.version,
                    WORKSPACE_MANIFEST_VERSION | MIGRATION_MANIFEST_VERSION
                ) =>
            {
                (value.generation, value.version)
            }
            Wire::Versioned(_) => {
                return Err(serde::de::Error::custom("unsupported manifest version"));
            }
            Wire::Legacy(value) => (value, 1),
        };
        if version == 1
            && manifest
                .modules
                .values()
                .flat_map(|state| &state.intents)
                .any(|intent| intent.action.requires_v2())
        {
            return Err(serde::de::Error::custom(
                "workspace actions require manifest version 2",
            ));
        }
        if version < MIGRATION_MANIFEST_VERSION
            && (!manifest.directory_transitions.is_empty() || manifest.migration.is_some())
        {
            return Err(serde::de::Error::custom(
                "migration receipts require manifest version 3",
            ));
        }
        for transition in &manifest.directory_transitions {
            transition.validate().map_err(serde::de::Error::custom)?;
        }
        Ok(Self {
            number: manifest.number,
            modules: manifest.modules,
            directory_transitions: manifest.directory_transitions,
            migration: manifest.migration,
        })
    }
}
