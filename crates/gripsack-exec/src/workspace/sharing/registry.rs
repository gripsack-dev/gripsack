//! Bounded OCI Distribution transport. No activation, trust, or fallback effects.
pub use super::closure::ClosureBundle as RegistryBundle;
use super::closure::{ExternalManifest, MAX_CLOSURE_MANIFESTS, MAX_CLOSURE_METADATA_BYTES};
use super::trust::{MAX_ENVELOPE_BYTES, MAX_MANIFEST_BYTES, PAYLOAD_TYPE, digest};
use reqwest::{
    StatusCode, Url,
    blocking::{Client, Response},
    header::{ACCEPT, CONTENT_TYPE, LOCATION},
    redirect::Policy,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    time::{Duration, Instant},
};

pub const MAX_PAYLOAD_BYTES: usize = 512 * 1024 * 1024;
const MAX_TRANSPORT_BYTES: usize = 1024 * 1024;
const MAX_DESCRIPTORS: usize = 4096;
const SESSION_TIMEOUT: Duration = Duration::from_secs(300);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const OCI_MANIFEST: &str = "application/vnd.oci.image.manifest.v1+json";
const ARTIFACT_TYPE: &str = "application/vnd.gripsack.portable.v1";
const ENVELOPE_TYPE: &str = "application/vnd.dsse.envelope.v1+json";
const PAYLOAD_BLOB_TYPE: &str = "application/vnd.gripsack.portable.blob.v1";
const EXTERNAL_MANIFEST_TYPE: &str = "application/vnd.gripsack.portable.external-manifest.v1+json";
const EXTERNAL_ENVELOPE_TYPE: &str = "application/vnd.gripsack.portable.external-envelope.v1+json";
const EMPTY_CONFIG_TYPE: &str = "application/vnd.oci.empty.v1+json";

#[derive(Debug, thiserror::Error)]
#[error("portable registry: {0}")]
pub struct RegistryError(String);
fn reject(message: &str) -> RegistryError {
    RegistryError(message.into())
}
/// Credentials are deliberately neither Debug nor Serialize. No ambient auth lookup.
pub enum RegistryAuth {
    Anonymous,
    Bearer(String),
    Basic { username: String, password: String },
}
/// One explicit transfer session, bounded from construction by SESSION_TIMEOUT.
pub struct Registry {
    client: Client,
    base: Url,
    repository: String,
    auth: RegistryAuth,
    deadline: Instant,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Descriptor {
    #[serde(rename = "mediaType")]
    media_type: String,
    digest: String,
    size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    annotations: Option<ExternalBinding>,
}
/// Binds the signature envelope to its exact independent manifest, not to any
/// publisher authority. Namespace authorization belongs to runtime admission.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExternalBinding {
    #[serde(rename = "org.gripsack.manifest")]
    manifest: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TransportManifest {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    #[serde(rename = "mediaType")]
    media_type: String,
    #[serde(rename = "artifactType")]
    artifact_type: String,
    config: Descriptor,
    layers: Vec<Descriptor>,
}
impl Descriptor {
    fn new(media_type: &str, bytes: &[u8]) -> Self {
        Self {
            media_type: media_type.into(),
            digest: format!("sha256:{}", digest(bytes)),
            size: bytes.len() as u64,
            annotations: None,
        }
    }
    fn external(media_type: &str, bytes: &[u8], manifest: &str) -> Self {
        let mut descriptor = Self::new(media_type, bytes);
        descriptor.annotations = Some(ExternalBinding {
            manifest: format!("sha256:{manifest}"),
        });
        descriptor
    }
    fn validate(&self, bytes: &[u8]) -> Result<(), RegistryError> {
        if self.size != bytes.len() as u64 || self.digest != format!("sha256:{}", digest(bytes)) {
            return Err(reject("blob digest or size mismatch"));
        }
        Ok(())
    }
}
impl Registry {
    /// Insecure HTTP is only explicit, loopback-only, and anonymous. Redirects
    /// are rejected even on HTTPS: neither credentials nor upload bytes follow them.
    pub fn new(
        base_url: &str,
        repository: &str,
        auth: RegistryAuth,
        allow_insecure_loopback: bool,
    ) -> Result<Self, RegistryError> {
        let deadline = Instant::now() + SESSION_TIMEOUT;
        let base = Url::parse(base_url).map_err(|_| reject("invalid registry URL"))?;
        let loopback = base.host_str().is_some_and(|host| {
            host == "localhost"
                || host == "[::1]"
                || host == "::1"
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        });
        if (base.scheme() != "https"
            && !(base.scheme() == "http"
                && allow_insecure_loopback
                && loopback
                && matches!(&auth, RegistryAuth::Anonymous)))
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
            || base.path() != "/"
            || base.host_str().is_none()
        {
            return Err(reject(
                "registry requires HTTPS origin without embedded credentials; HTTP requires explicit anonymous loopback opt-in",
            ));
        }
        if repository.is_empty()
            || repository.len() > 255
            || repository.split('/').any(|part| {
                part.is_empty()
                    || !part.bytes().all(|b| {
                        b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b)
                    })
                    || !part.as_bytes()[0].is_ascii_alphanumeric()
            })
        {
            return Err(reject("invalid OCI repository name"));
        }
        let client = crate::http_client::builder()
            .redirect(Policy::none())
            .https_only(base.scheme() == "https")
            .connect_timeout(Duration::from_secs(10))
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|_| reject("cannot initialize registry HTTP client"))?;
        Ok(Self {
            client,
            base,
            repository: repository.into(),
            auth,
            deadline,
        })
    }
    fn url(&self, suffix: &str) -> Result<Url, RegistryError> {
        self.base
            .join(&format!("v2/{}/{suffix}", self.repository))
            .map_err(|_| reject("invalid registry endpoint"))
    }
    fn request(
        &self,
        method: reqwest::Method,
        url: Url,
    ) -> Result<reqwest::blocking::RequestBuilder, RegistryError> {
        let remaining = self
            .deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| reject("registry transfer session deadline exceeded"))?;
        let request = self
            .client
            .request(method, url)
            .timeout(remaining.min(REQUEST_TIMEOUT));
        Ok(match &self.auth {
            RegistryAuth::Anonymous => request,
            RegistryAuth::Bearer(token) => request.bearer_auth(token),
            RegistryAuth::Basic { username, password } => {
                request.basic_auth(username, Some(password))
            }
        })
    }
    fn get(
        &self,
        url: Url,
        bound: usize,
        media_type: Option<&str>,
    ) -> Result<Vec<u8>, RegistryError> {
        let mut request = self.request(reqwest::Method::GET, url)?;
        if let Some(media_type) = media_type {
            request = request.header(ACCEPT, media_type);
        }
        let response = request
            .send()
            .map_err(|_| reject("registry request failed"))?;
        if response.status() != StatusCode::OK {
            return Err(status_error(response.status()));
        }
        if let Some(expected) = media_type
            && response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.split(';').next())
                != Some(expected)
        {
            return Err(reject("registry response media type mismatch"));
        }
        read_response(response, bound)
    }
    fn blob(&self, descriptor: &Descriptor, bound: usize) -> Result<Vec<u8>, RegistryError> {
        require_digest(&descriptor.digest)?;
        if descriptor.size > bound as u64 {
            return Err(reject("blob exceeds transfer budget"));
        }
        let bytes = self.get(
            self.url(&format!("blobs/{}", descriptor.digest))?,
            descriptor.size as usize,
            None,
        )?;
        descriptor.validate(&bytes)?;
        Ok(bytes)
    }
    /// Pull is digest-only. Mutable tag discovery is not a frozen-consumption API.
    pub fn pull(&self, transport_digest: &str) -> Result<RegistryBundle, RegistryError> {
        require_digest(transport_digest)?;
        let bytes = self.get(
            self.url(&format!("manifests/{transport_digest}"))?,
            MAX_TRANSPORT_BYTES,
            Some(OCI_MANIFEST),
        )?;
        if format!("sha256:{}", digest(&bytes)) != transport_digest {
            return Err(reject("transport manifest digest mismatch"));
        }
        let transport: TransportManifest =
            serde_json::from_slice(&bytes).map_err(|_| reject("invalid OCI artifact manifest"))?;
        if transport.schema_version != 2
            || transport.media_type != OCI_MANIFEST
            || transport.artifact_type != ARTIFACT_TYPE
            || transport.config.media_type != EMPTY_CONFIG_TYPE
            || transport.layers.len() < 2
            || transport.layers.len() > MAX_DESCRIPTORS
            || transport.layers[0].media_type != PAYLOAD_TYPE
            || transport.layers[1].media_type != ENVELOPE_TYPE
        {
            return Err(reject("unsupported OCI artifact structure"));
        }
        // Reject the complete advertised transfer budget before allocating or
        // fetching any component. Metadata counts against the same total as blobs.
        let mut remaining = MAX_PAYLOAD_BYTES
            .checked_sub(bytes.len())
            .ok_or_else(|| reject("bundle exceeds total transfer byte budget"))?;
        let mut metadata_remaining = MAX_CLOSURE_METADATA_BYTES;
        let mut manifest_count = 0;
        for descriptor in std::iter::once(&transport.config).chain(&transport.layers) {
            require_digest(&descriptor.digest)?;
            let size = usize::try_from(descriptor.size)
                .map_err(|_| reject("descriptor exceeds transfer byte budget"))?;
            remaining = remaining
                .checked_sub(size)
                .ok_or_else(|| reject("bundle exceeds total transfer byte budget"))?;
            let metadata_bound = match descriptor.media_type.as_str() {
                PAYLOAD_TYPE | EXTERNAL_MANIFEST_TYPE => {
                    manifest_count += 1;
                    Some(MAX_MANIFEST_BYTES)
                }
                ENVELOPE_TYPE | EXTERNAL_ENVELOPE_TYPE => Some(MAX_ENVELOPE_BYTES),
                _ => None,
            };
            if let Some(bound) = metadata_bound {
                if size > bound || manifest_count > MAX_CLOSURE_MANIFESTS {
                    return Err(reject("bundle exceeds metadata or manifest count budget"));
                }
                metadata_remaining = metadata_remaining
                    .checked_sub(size)
                    .ok_or_else(|| reject("bundle exceeds total metadata byte budget"))?;
            }
        }
        if transport.config.annotations.is_some()
            || transport.layers[0].annotations.is_some()
            || transport.layers[1].annotations.is_some()
        {
            return Err(reject("unexpected external binding on root descriptor"));
        }
        if self.blob(&transport.config, 2)? != b"{}" {
            return Err(reject("invalid OCI empty configuration"));
        }
        let manifest = self.blob(&transport.layers[0], MAX_MANIFEST_BYTES)?;
        let envelope = self.blob(&transport.layers[1], MAX_ENVELOPE_BYTES)?;
        let mut external_manifests = BTreeMap::new();
        let mut blobs = BTreeMap::new();
        let mut descriptors = transport.layers[2..].iter();
        while let Some(descriptor) = descriptors.next() {
            if descriptor.media_type == EXTERNAL_MANIFEST_TYPE && blobs.is_empty() {
                let binding = descriptor
                    .annotations
                    .as_ref()
                    .ok_or_else(|| reject("external manifest lacks digest binding"))?;
                if binding.manifest != descriptor.digest {
                    return Err(reject("external manifest binding mismatch"));
                }
                let signature = descriptors
                    .next()
                    .ok_or_else(|| reject("missing external signature descriptor"))?;
                if signature.media_type != EXTERNAL_ENVELOPE_TYPE
                    || signature.annotations.as_ref().map(|value| &value.manifest)
                        != Some(&binding.manifest)
                {
                    return Err(reject("external signature binding mismatch"));
                }
                let identity = descriptor.digest[7..].to_owned();
                if external_manifests.contains_key(&identity) {
                    return Err(reject("duplicate external manifest descriptor"));
                }
                external_manifests.insert(
                    identity,
                    ExternalManifest {
                        manifest: self.blob(descriptor, MAX_MANIFEST_BYTES)?,
                        envelope: self.blob(signature, MAX_ENVELOPE_BYTES)?,
                    },
                );
            } else if descriptor.media_type == PAYLOAD_BLOB_TYPE && descriptor.annotations.is_none()
            {
                let identity = descriptor.digest[7..].to_owned();
                if blobs.contains_key(&identity) {
                    return Err(reject("duplicate payload descriptor"));
                }
                blobs.insert(identity, self.blob(descriptor, MAX_PAYLOAD_BYTES)?);
            } else {
                return Err(reject(
                    "unsupported payload descriptor media type or ordering",
                ));
            }
        }
        let bundle = RegistryBundle {
            manifest,
            envelope,
            external_manifests,
            blobs,
        };
        bundle
            .validate()
            .map_err(|error| RegistryError(error.to_string()))?;
        if Instant::now() >= self.deadline {
            return Err(reject("registry transfer session deadline exceeded"));
        }
        Ok(bundle)
    }
    /// Consumes buffers so uploads do not duplicate the bounded payload in memory.
    /// Blobs precede manifest publication. Failure after PUT may have unknown remote
    /// outcome; this API never claims tag compare-and-swap or rolls back a tag.
    pub fn push(&self, reference: &str, bundle: RegistryBundle) -> Result<String, RegistryError> {
        validate_reference(reference)?;
        bundle
            .validate()
            .map_err(|error| RegistryError(error.to_string()))?;
        if bundle.blobs.len() + 2 * bundle.external_manifests.len() + 2 > MAX_DESCRIPTORS {
            return Err(reject("bundle exceeds metadata or descriptor budget"));
        }
        let config = Descriptor::new(EMPTY_CONFIG_TYPE, b"{}");
        let mut layers = vec![
            Descriptor::new(PAYLOAD_TYPE, &bundle.manifest),
            Descriptor::new(ENVELOPE_TYPE, &bundle.envelope),
        ];
        for (identity, external) in &bundle.external_manifests {
            layers.push(Descriptor::external(
                EXTERNAL_MANIFEST_TYPE,
                &external.manifest,
                identity,
            ));
            layers.push(Descriptor::external(
                EXTERNAL_ENVELOPE_TYPE,
                &external.envelope,
                identity,
            ));
        }
        layers.extend(
            bundle
                .blobs
                .values()
                .map(|bytes| Descriptor::new(PAYLOAD_BLOB_TYPE, bytes)),
        );
        let transport = TransportManifest {
            schema_version: 2,
            media_type: OCI_MANIFEST.into(),
            artifact_type: ARTIFACT_TYPE.into(),
            config,
            layers,
        };
        let bytes = serde_json::to_vec(&transport)
            .map_err(|_| reject("cannot encode transport manifest"))?;
        if bytes.len() > MAX_TRANSPORT_BYTES {
            return Err(reject("transport manifest exceeds byte budget"));
        }
        let mut remaining = MAX_PAYLOAD_BYTES
            .checked_sub(bytes.len())
            .ok_or_else(|| reject("bundle exceeds total transfer byte budget"))?;
        for descriptor in std::iter::once(&transport.config).chain(&transport.layers) {
            remaining = remaining
                .checked_sub(descriptor.size as usize)
                .ok_or_else(|| reject("bundle exceeds total transfer byte budget"))?;
        }
        let transport_digest = format!("sha256:{}", digest(&bytes));
        if reference.starts_with("sha256:") && reference != transport_digest {
            return Err(reject("publication digest does not match transport bytes"));
        }
        let mut uploaded = BTreeSet::new();
        self.upload(&transport.config, b"{}".to_vec())?;
        for (descriptor, bytes) in transport.layers.iter().zip(
            std::iter::once(bundle.manifest)
                .chain(std::iter::once(bundle.envelope))
                .chain(
                    bundle
                        .external_manifests
                        .into_values()
                        .flat_map(|external| [external.manifest, external.envelope]),
                )
                .chain(bundle.blobs.into_values()),
        ) {
            if uploaded.insert(&descriptor.digest) {
                self.upload(descriptor, bytes)?;
            }
        }
        let response = self
            .request(
                reqwest::Method::PUT,
                self.url(&format!("manifests/{reference}"))?,
            )?
            .header(CONTENT_TYPE, OCI_MANIFEST)
            .body(bytes)
            .send()
            .map_err(|_| reject("manifest publication failed; remote outcome unknown"))?;
        if response.status() != StatusCode::CREATED {
            return Err(status_error(response.status()));
        }
        if let Some(returned) = response.headers().get("Docker-Content-Digest")
            && returned.to_str().ok() != Some(transport_digest.as_str())
        {
            return Err(reject("registry returned conflicting transport digest"));
        }
        Ok(transport_digest)
    }
    fn upload(&self, descriptor: &Descriptor, bytes: Vec<u8>) -> Result<(), RegistryError> {
        let response = self
            .request(reqwest::Method::POST, self.url("blobs/uploads/")?)?
            .header("Content-Length", "0")
            .send()
            .map_err(|_| reject("cannot start blob upload"))?;
        if response.status() != StatusCode::ACCEPTED {
            return Err(status_error(response.status()));
        }
        let location = response
            .headers()
            .get(LOCATION)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| reject("upload response lacks location"))?;
        let mut url = self
            .url("blobs/uploads/")?
            .join(location)
            .map_err(|_| reject("invalid upload location"))?;
        if url.origin() != self.base.origin()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || !url
                .path()
                .starts_with(&format!("/v2/{}/blobs/uploads/", self.repository))
            || url.query_pairs().any(|(key, _)| key == "digest")
        {
            return Err(reject("registry upload location escaped repository/origin"));
        }
        url.query_pairs_mut()
            .append_pair("digest", &descriptor.digest);
        let response = self
            .request(reqwest::Method::PUT, url)?
            .header(CONTENT_TYPE, "application/octet-stream")
            .body(bytes)
            .send()
            .map_err(|_| reject("blob upload failed"))?;
        if response.status() != StatusCode::CREATED {
            return Err(status_error(response.status()));
        }
        Ok(())
    }
}
fn status_error(status: StatusCode) -> RegistryError {
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        reject("registry authentication/authorization rejected; no fallback")
    } else {
        RegistryError(format!(
            "unexpected registry HTTP status {}",
            status.as_u16()
        ))
    }
}
fn read_response(response: Response, bound: usize) -> Result<Vec<u8>, RegistryError> {
    if response
        .content_length()
        .is_some_and(|size| size > bound as u64)
    {
        return Err(reject("HTTP body exceeds byte budget"));
    }
    let mut bytes = Vec::new();
    response
        .take(bound as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| reject("incomplete registry response"))?;
    if bytes.len() > bound {
        return Err(reject("HTTP body exceeds byte budget"));
    }
    Ok(bytes)
}
fn valid_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn require_digest(value: &str) -> Result<(), RegistryError> {
    if !value.strip_prefix("sha256:").is_some_and(valid_hex) {
        return Err(reject("expected exact lowercase sha256 digest reference"));
    }
    Ok(())
}
fn validate_reference(value: &str) -> Result<(), RegistryError> {
    if value.starts_with("sha256:") {
        return require_digest(value);
    }
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
        || value.starts_with(['.', '-'])
    {
        return Err(reject("invalid OCI publication reference"));
    }
    Ok(())
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
