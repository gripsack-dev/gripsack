//! Transport-independent, canonical passive package closure. Local member IDs
//! allow cycles without self-referential manifest digests. No automation DTO is
//! accepted here; every supported field has passive data semantics.
use super::target::PortableTarget;
use crate::ExecError;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const FORMAT_VERSION: u32 = 1;
pub const MAX_MANIFEST_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_COMPONENTS: usize = 256;
pub const MAX_ENTRIES: usize = 100_000;
pub const MAX_PAYLOAD_BYTES: u64 = 512 * 1024 * 1024;
pub const TREE_MEDIA_TYPE: &str = "application/vnd.gripsack.tree.v1.tar";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortableManifest {
    pub version: u32,
    pub namespace: String,
    pub publisher: String,
    pub target: PortableTarget,
    pub layout: RuntimeLayout,
    pub components: Vec<Component>,
    pub roots: Vec<String>,
    pub commands: BTreeMap<String, PortableCommand>,
    pub environment: BTreeMap<String, PortableEnvironmentValue>,
    pub licenses: Vec<String>,
    pub redistribution_authorized: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RuntimeLayout {
    Relocatable {
        contract: RuntimeContract,
    },
    Fixed {
        prefix: String,
        contract: RuntimeContract,
    },
    Conda,
    /// Ordered passive package selection; members retain independent prefixes.
    Environment {},
}
impl RuntimeLayout {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Relocatable { .. } => "relocatable",
            Self::Fixed { .. } => "fixed",
            Self::Conda => "conda",
            Self::Environment {} => "environment",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeContract {
    /// Publisher's explicit qualified behavior; not an executable validator.
    pub behavior: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Component {
    pub id: String,
    pub payload: Option<BlobDescriptor>,
    pub tree: Option<String>,
    pub runtime: Vec<String>,
    pub conda: Option<super::conda::PortableConda>,
    pub external: Option<ExternalReference>,
    pub inventory: Vec<PortableEntry>,
}
/// Exact independent package authority, not a namespace claim by its parent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalReference {
    pub manifest: String,
    pub namespace: String,
    pub publisher: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlobDescriptor {
    pub sha256: String,
    pub size: u64,
    pub media_type: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortableCommand {
    pub component: String,
    pub executable: String,
    pub args: Vec<String>,
}
/// The existing native environment argument subset, with component-local
/// artifact authority rather than repository inputs or arbitrary host paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PortableEnvironmentValue {
    Literal { value: String },
    Artifact { output: String, selector: String },
}
impl From<PortableEnvironmentValue> for gripsack_ir::workspace_model::WorkspaceArg {
    fn from(value: PortableEnvironmentValue) -> Self {
        match value {
            PortableEnvironmentValue::Literal { value } => Self::Literal { value },
            PortableEnvironmentValue::Artifact { output, selector } => {
                Self::Artifact { output, selector }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortableEntry {
    pub path: String,
    pub mode: u32,
    pub kind: EntryKind,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EntryKind {
    File { sha256: String, size: u64 },
    Directory,
    Symlink { target: String },
}

pub(super) fn failure(detail: impl Into<String>) -> ExecError {
    ExecError::Step {
        output: "package".into(),
        step: "portable admission".into(),
        detail: detail.into(),
    }
}
pub(super) fn digest(value: &str) -> Result<(), ExecError> {
    gripsack_process::Sha256Digest::parse(value)
        .map(|_| ())
        .map_err(Into::into)
}
pub(super) fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        && value != "."
        && value != ".."
}
/// V1 deliberately accepts UTF-8's printable ASCII subset. Backslashes,
/// control characters and ambiguous dot components are rejected, not repaired.
pub(super) fn relative_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 4096
        && path.bytes().all(|b| (32..127).contains(&b) && b != b'\\')
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}
impl PortableManifest {
    pub fn decode(bytes: &[u8]) -> Result<Self, ExecError> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(failure("portable manifest exceeds byte bound"));
        }
        let value: Self = serde_json::from_slice(bytes)?;
        // A single encoding also rejects duplicate keys, reordered fields,
        // insignificant whitespace and alternative escaped-string spellings.
        if serde_json::to_vec(&value)? != bytes {
            return Err(failure("portable manifest is not canonical JSON"));
        }
        value.validate()?;
        Ok(value)
    }
    pub fn encode(&self) -> Result<Vec<u8>, ExecError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self)?;
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(failure("portable manifest exceeds byte bound"));
        }
        Ok(bytes)
    }
    pub fn digest(&self) -> Result<String, ExecError> {
        Ok(gripsack_store::hash::hex_sha256(&self.encode()?))
    }
    pub fn validate(&self) -> Result<(), ExecError> {
        if self.version != FORMAT_VERSION {
            return Err(failure("unsupported portable format (requires v1)"));
        }
        self.target.validate()?;
        if !self.namespace.split('/').all(identifier)
            || self.namespace.len() > 256
            || !identifier(&self.publisher)
        {
            return Err(failure("invalid package namespace or publisher"));
        }
        if !self.redistribution_authorized {
            return Err(failure(
                "publication requires explicit redistribution authorization",
            ));
        }
        if self.components.is_empty()
            || self.components.len() > MAX_COMPONENTS
            || self.roots.is_empty()
            || self.roots.len() > MAX_COMPONENTS
            || self.roots.iter().collect::<BTreeSet<_>>().len() != self.roots.len()
        {
            return Err(failure("portable component/root count outside bound"));
        }
        match &self.layout {
            RuntimeLayout::Fixed { prefix, contract } => {
                if !prefix.starts_with('/') || !relative_path(&prefix[1..]) {
                    return Err(failure("fixed layout requires canonical absolute prefix"));
                }
                validate_contract(contract)?;
            }
            RuntimeLayout::Relocatable { contract } => validate_contract(contract)?,
            RuntimeLayout::Conda | RuntimeLayout::Environment {} => {}
        }
        let mut components = BTreeMap::new();
        let mut total_bytes = 0u64;
        let mut total_entries = 0usize;
        for component in &self.components {
            if !identifier(&component.id)
                || components
                    .insert(component.id.as_str(), component)
                    .is_some()
            {
                return Err(failure("invalid or duplicate component ID"));
            }
            if component.runtime.len() > MAX_COMPONENTS {
                return Err(failure("runtime edge count exceeds bound"));
            }
            match (
                &component.payload,
                &component.tree,
                &component.conda,
                &component.external,
            ) {
                (Some(blob), Some(tree), None, None) => {
                    if matches!(self.layout, RuntimeLayout::Conda) {
                        return Err(failure(
                            "Conda layout requires original archives, not a tree payload",
                        ));
                    }
                    blob.validate()?;
                    if blob.media_type != TREE_MEDIA_TYPE {
                        return Err(failure("unsupported portable tree media type"));
                    }
                    digest(tree)?;
                    total_bytes = total_bytes
                        .checked_add(blob.size)
                        .ok_or_else(|| failure("payload total overflow"))?;
                }
                (None, None, Some(conda), None) => {
                    if !matches!(
                        self.layout,
                        RuntimeLayout::Conda | RuntimeLayout::Environment {}
                    ) || (matches!(self.layout, RuntimeLayout::Conda)
                        && self.components.len() != 1)
                        || !component.inventory.is_empty()
                    {
                        return Err(failure(
                            "Conda members require a Conda or passive environment layout and no installed tree",
                        ));
                    }
                    conda.validate()?;
                    for blob in conda.descriptors() {
                        blob.validate()?;
                        total_bytes = total_bytes
                            .checked_add(blob.size)
                            .ok_or_else(|| failure("payload total overflow"))?;
                    }
                }
                (None, None, None, Some(external)) => {
                    digest(&external.manifest)?;
                    if !external.namespace.split('/').all(identifier)
                        || external.namespace.len() > 256
                        || !identifier(&external.publisher)
                        || !component.inventory.is_empty()
                        || matches!(self.layout, RuntimeLayout::Conda)
                    {
                        return Err(failure(
                            "invalid independently published component reference",
                        ));
                    }
                }
                _ => {
                    return Err(failure(
                        "component requires exactly one local tree, Conda archive set or independent manifest",
                    ));
                }
            }
            total_entries = total_entries
                .checked_add(component.inventory.len())
                .ok_or_else(|| failure("inventory count overflow"))?;
            let mut paths = BTreeSet::new();
            let mut previous: Option<&str> = None;
            for entry in &component.inventory {
                if !relative_path(&entry.path)
                    || previous.is_some_and(|p| p >= entry.path.as_str())
                    || !paths.insert(entry.path.to_ascii_lowercase())
                {
                    return Err(failure(
                        "inventory paths must be sorted, unique and case-unambiguous",
                    ));
                }
                previous = Some(&entry.path);
                if entry.mode & !0o777 != 0 || entry.mode & 0o022 != 0 {
                    return Err(failure(
                        "portable modes reject privileged or group/world-writable entries",
                    ));
                }
                match &entry.kind {
                    EntryKind::File { sha256, size } => {
                        digest(sha256)?;
                        if *size > MAX_PAYLOAD_BYTES {
                            return Err(failure("inventory file exceeds byte bound"));
                        }
                    }
                    EntryKind::Directory => {}
                    EntryKind::Symlink { target } => {
                        // Conservative component-local links; archive extraction
                        // independently checks composed links and containment.
                        if !relative_path(target) {
                            return Err(failure("symlink target is not a supported relative path"));
                        }
                    }
                }
            }
        }
        if total_bytes > MAX_PAYLOAD_BYTES || total_entries > MAX_ENTRIES {
            return Err(failure("portable closure exceeds byte/entry bound"));
        }
        for component in &self.components {
            let mut seen = BTreeSet::new();
            for dependency in &component.runtime {
                if !components.contains_key(dependency.as_str()) || !seen.insert(dependency) {
                    return Err(failure("missing or duplicate runtime member"));
                }
            }
        }
        let mut reached = BTreeSet::new();
        let mut pending: Vec<_> = self.roots.iter().collect();
        while let Some(id) = pending.pop() {
            let component = components
                .get(id.as_str())
                .ok_or_else(|| failure("selected root is absent"))?;
            if reached.insert(id) {
                pending.extend(component.runtime.iter());
            }
        }
        if reached.len() != components.len() {
            return Err(failure("manifest contains unselected runtime components"));
        }
        if self.commands.len() > 1024 || self.environment.len() > 256 {
            return Err(failure("command/environment count exceeds bound"));
        }
        for (name, command) in &self.commands {
            if !identifier(name)
                || !components.contains_key(command.component.as_str())
                || !relative_path(&command.executable)
                || command.args.len() > 256
                || command
                    .args
                    .iter()
                    .any(|arg| arg.len() > 4096 || arg.contains('\0'))
            {
                return Err(failure("invalid passive command contract"));
            }
        }
        for (name, value) in &self.environment {
            if !crate::env::is_identifier(name)
                || name == "PATH"
                || name.starts_with("LD_")
                || name.starts_with("GRIPSACK_")
                || name.starts_with("DENO_")
                || matches!(
                    name.as_str(),
                    "GLIBC_TUNABLES"
                        | "GCONV_PATH"
                        | "PYTHONPATH"
                        | "PYTHONHOME"
                        | "PYTHONSTARTUP"
                        | "PYTHONINSPECT"
                        | "BASH_ENV"
                        | "ENV"
                        | "SHELLOPTS"
                        | "BASHOPTS"
                        | "IFS"
                        | "CDPATH"
                        | "NODE_OPTIONS"
                        | "PERL5OPT"
                        | "RUBYOPT"
                )
            {
                return Err(failure(
                    "portable environment names cannot grant host loader, interpreter or shell control",
                ));
            }
            match value {
                PortableEnvironmentValue::Literal { value } => {
                    if value.len() > 4096 || value.contains(['\0', '/', '\\']) {
                        return Err(failure(
                            "portable literals must be bounded and path-free; use an artifact binding for a managed path",
                        ));
                    }
                }
                PortableEnvironmentValue::Artifact { output, selector } => {
                    if !components.contains_key(output.as_str())
                        || (selector != "." && !relative_path(selector))
                    {
                        return Err(failure(
                            "environment artifact must select an admitted local component and safe relative path",
                        ));
                    }
                }
            }
        }
        if self.licenses.len() > MAX_ENTRIES
            || self.licenses.iter().any(|path| !relative_path(path))
        {
            return Err(failure("invalid license path inventory"));
        }
        Ok(())
    }
}
impl BlobDescriptor {
    pub fn validate(&self) -> Result<(), ExecError> {
        digest(&self.sha256)?;
        if self.size > MAX_PAYLOAD_BYTES
            || self.media_type.is_empty()
            || self.media_type.len() > 128
        {
            return Err(failure("blob descriptor exceeds bounds"));
        }
        Ok(())
    }
}
fn validate_contract(contract: &RuntimeContract) -> Result<(), ExecError> {
    if contract.behavior.trim().is_empty()
        || contract.behavior.len() > 8192
        || contract.behavior.contains('\0')
    {
        return Err(failure(
            "runtime contract must state bounded behavior; machine constraints belong in target",
        ));
    }
    Ok(())
}
