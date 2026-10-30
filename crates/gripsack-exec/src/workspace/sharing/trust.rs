//! Exact-byte DSSE verification and explicitly local publisher authority.
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs::OpenOptions,
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
};
use zeroize::Zeroizing;

pub const PAYLOAD_TYPE: &str = "application/vnd.gripsack.portable.v1+json";
pub const MAX_MANIFEST_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_ENVELOPE_BYTES: usize = 6 * 1024 * 1024;
const MAX_POLICY_BYTES: usize = 1024 * 1024;
const KNOWN_VALIDATORS: &[&str] = &["manifest-integrity", "payload-integrity", "layout-safety"];

#[derive(Debug, thiserror::Error)]
#[error("portable trust: {0}")]
pub struct TrustError(String);
fn reject(message: &str) -> TrustError {
    TrustError(message.into())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    #[serde(rename = "payloadType")]
    payload_type: String,
    payload: String,
    signatures: Vec<EnvelopeSignature>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnvelopeSignature {
    keyid: String,
    sig: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyWire {
    version: u32,
    keys: Vec<PublisherKey>,
    revoked_keys: BTreeSet<String>,
    revoked_manifests: BTreeSet<String>,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublisherKey {
    key_id: String,
    publisher: String,
    namespaces: BTreeSet<String>,
    public_key: String,
    allowed_layouts: BTreeSet<String>,
    required_validators: BTreeSet<String>,
}

pub struct TrustPolicy {
    policy: PolicyWire,
    revision: String,
}
/// Cryptographic verification is not canonical-format or namespace admission.
/// Callers must admit the exact input bytes, then authorize their parsed claims.
pub struct VerifiedSignature {
    key: PublisherKey,
    revision: String,
    manifest_digest: String,
}
impl VerifiedSignature {
    pub fn publisher(&self) -> &str {
        &self.key.publisher
    }
    pub fn key_id(&self) -> &str {
        &self.key.key_id
    }
    pub fn revision(&self) -> &str {
        &self.revision
    }
    pub fn manifest_digest(&self) -> &str {
        &self.manifest_digest
    }
    pub fn authorize(&self, namespace: &str, publisher: &str) -> Result<(), TrustError> {
        if publisher != self.key.publisher || !self.key.namespaces.contains(namespace) {
            return Err(reject("publisher is not authorized for this namespace"));
        }
        Ok(())
    }
    /// Check declared layouts before any approved materializer is invoked.
    /// This does not assert that required validation has completed.
    pub fn authorize_requested_layouts(&self, layouts: &[&str]) -> Result<(), TrustError> {
        if layouts
            .iter()
            .any(|layout| !self.key.allowed_layouts.contains(*layout))
        {
            return Err(reject("layout is not authorized by local policy"));
        }
        Ok(())
    }
    pub fn authorize_layouts(
        &self,
        layouts: &[&str],
        completed_validators: &[&str],
    ) -> Result<(), TrustError> {
        self.authorize_requested_layouts(layouts)?;
        if self
            .key
            .required_validators
            .iter()
            .any(|validator| !completed_validators.contains(&validator.as_str()))
        {
            return Err(reject("required local validator has not completed"));
        }
        Ok(())
    }
}
impl TrustPolicy {
    /// Missing policy denies by returning an error, never creating trust roots.
    pub fn load(home: &Path) -> Result<Self, TrustError> {
        let bytes = read_bounded(&home.join("sharing/trust.json"), MAX_POLICY_BYTES, false)?;
        Self::from_bytes(&bytes)
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, TrustError> {
        if bytes.len() > MAX_POLICY_BYTES {
            return Err(reject("policy exceeds byte limit"));
        }
        let policy: PolicyWire =
            serde_json::from_slice(bytes).map_err(|_| reject("invalid local trust policy"))?;
        if policy.version != 1 {
            return Err(reject("unsupported trust policy version"));
        }
        let mut ids = BTreeSet::new();
        for key in &policy.keys {
            if key.key_id.is_empty()
                || key.publisher.is_empty()
                || !ids.insert(&key.key_id)
                || key.namespaces.iter().any(String::is_empty)
                || key
                    .required_validators
                    .iter()
                    .any(|v| !KNOWN_VALIDATORS.contains(&v.as_str()))
            {
                return Err(reject("invalid or unsupported publisher policy"));
            }
            public_key(&key.public_key)?;
        }
        Ok(Self {
            policy,
            revision: digest(bytes),
        })
    }
    pub fn revision(&self) -> &str {
        &self.revision
    }
    pub fn verify(
        &self,
        manifest: &[u8],
        envelope: &[u8],
    ) -> Result<VerifiedSignature, TrustError> {
        if manifest.len() > MAX_MANIFEST_BYTES || envelope.len() > MAX_ENVELOPE_BYTES {
            return Err(reject("signed input exceeds byte limit"));
        }
        let envelope: Envelope =
            serde_json::from_slice(envelope).map_err(|_| reject("invalid DSSE envelope"))?;
        if envelope.payload_type != PAYLOAD_TYPE || envelope.signatures.len() != 1 {
            return Err(reject("unsupported DSSE payload type or signature count"));
        }
        let payload = decode(&envelope.payload)?;
        if payload != manifest {
            return Err(reject("DSSE payload does not match exact manifest bytes"));
        }
        let manifest_digest = digest(manifest);
        if self.policy.revoked_manifests.contains(&manifest_digest) {
            return Err(reject("manifest is locally revoked"));
        }
        let signature = &envelope.signatures[0];
        if self.policy.revoked_keys.contains(&signature.keyid) {
            return Err(reject("publisher key is locally revoked"));
        }
        let key = self
            .policy
            .keys
            .iter()
            .find(|key| key.key_id == signature.keyid)
            .ok_or_else(|| reject("publisher key is not configured"))?;
        let sig = Signature::from_slice(&decode(&signature.sig)?)
            .map_err(|_| reject("invalid Ed25519 signature encoding"))?;
        public_key(&key.public_key)?
            .verify_strict(&preauth(manifest), &sig)
            .map_err(|_| reject("Ed25519 signature verification failed"))?;
        Ok(VerifiedSignature {
            key: key.clone(),
            revision: self.revision.clone(),
            manifest_digest,
        })
    }
}
fn public_key(encoded: &str) -> Result<VerifyingKey, TrustError> {
    let raw: [u8; 32] = decode(encoded)?
        .try_into()
        .map_err(|_| reject("invalid Ed25519 public key length"))?;
    let key = VerifyingKey::from_bytes(&raw).map_err(|_| reject("invalid Ed25519 public key"))?;
    if key.is_weak() {
        return Err(reject("weak Ed25519 public key"));
    }
    Ok(key)
}
fn decode(value: &str) -> Result<Vec<u8>, TrustError> {
    let raw = STANDARD
        .decode(value)
        .map_err(|_| reject("invalid base64 encoding"))?;
    if STANDARD.encode(&raw) != value {
        return Err(reject("noncanonical base64 encoding"));
    }
    Ok(raw)
}
fn preauth(manifest: &[u8]) -> Vec<u8> {
    let prefix = format!(
        "DSSEv1 {} {} {} ",
        PAYLOAD_TYPE.len(),
        PAYLOAD_TYPE,
        manifest.len()
    );
    let mut bytes = Vec::with_capacity(prefix.len() + manifest.len());
    bytes.extend_from_slice(prefix.as_bytes());
    bytes.extend_from_slice(manifest);
    bytes
}
pub(crate) fn digest(bytes: &[u8]) -> String {
    gripsack_process::Sha256Digest::of(bytes).to_string()
}
fn read_bounded(path: &Path, bound: usize, private: bool) -> Result<Vec<u8>, TrustError> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| reject("cannot open local trust/key file"))?;
    let metadata = file
        .metadata()
        .map_err(|_| reject("cannot inspect local trust/key file"))?;
    if !metadata.is_file() || metadata.len() > bound as u64 {
        return Err(reject("local trust/key file is not a bounded regular file"));
    }
    // Inspect the opened inode, not a pathname susceptible to replacement.
    // SAFETY: geteuid takes no arguments, dereferences no pointers, and has no
    // caller preconditions; it only observes the process effective user ID.
    if private
        && (metadata.mode() & 0o7777 != 0o600 || metadata.uid() != unsafe { libc::geteuid() })
    {
        return Err(reject(
            "private key must be owned by current user with mode 0600",
        ));
    }
    let mut bytes = Vec::new();
    file.take(bound as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| reject("cannot read local trust/key file"))?;
    if bytes.len() > bound {
        return Err(reject("local trust/key file exceeds byte limit"));
    }
    Ok(bytes)
}
/// The explicitly selected file is exactly a raw 32-byte seed, not a PEM or hex file.
pub fn sign(manifest: &[u8], key_path: &Path, key_id: &str) -> Result<Vec<u8>, TrustError> {
    if manifest.len() > MAX_MANIFEST_BYTES || key_id.is_empty() || key_id.len() > 256 {
        return Err(reject("manifest or key identifier exceeds signing bounds"));
    }
    let seed = Zeroizing::new(read_bounded(key_path, 32, true)?);
    let seed_array: Zeroizing<[u8; 32]> = Zeroizing::new(
        seed.as_slice()
            .try_into()
            .map_err(|_| reject("private seed must contain exactly 32 bytes"))?,
    );
    let key = SigningKey::from_bytes(&seed_array);
    let signature = key.sign(&preauth(manifest));
    serde_json::to_vec(&Envelope {
        payload_type: PAYLOAD_TYPE.into(),
        payload: STANDARD.encode(manifest),
        signatures: vec![EnvelopeSignature {
            keyid: key_id.into(),
            sig: STANDARD.encode(signature.to_bytes()),
        }],
    })
    .map_err(|_| reject("cannot encode DSSE envelope"))
}

#[cfg(test)]
#[path = "trust_tests.rs"]
mod tests;
