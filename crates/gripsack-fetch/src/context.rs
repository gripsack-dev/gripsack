//! One acquisition context per command/environment phase.

use crate::build_env::BuildProcessEnv;
use crate::fetch::{archive, brew, file, git, pixi, plugin, tarball};
use crate::limits::AcquisitionGate;
use crate::{DownloadHash, FetchError, FetchIdentity, FetchLimits, FetchOutcome};
use gripsack_ir::FetchSpec;
use serde::de::DeserializeOwned;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

pub struct FetchContext {
    limits: FetchLimits,
    network: crate::http::Client,
    build_env: BuildProcessEnv,
    acquisitions: AcquisitionGate,
    provisioning: Option<std::sync::Arc<FetchContext>>,
    /// Platform facts for bottle-tag selection (A0-01): detected once
    /// here, injected into the pure policy — the policy itself never
    /// reads the environment.
    host_platform: crate::bottle::HostPlatform,
}

impl Default for FetchContext {
    fn default() -> Self {
        Self::new(FetchLimits::default())
    }
}

impl FetchContext {
    pub fn new(limits: FetchLimits) -> Self {
        Self::with_build_env(limits, BuildProcessEnv::default())
    }

    /// Trusted-tool acquisition from an already admitted operator snapshot.
    /// Missing proxy/CA settings never fall back to a later ambient environment.
    pub fn from_operator(
        limits: FetchLimits,
        environment: &gripsack_process::OperatorEnvironment,
    ) -> Self {
        Self::with_build_env(limits, BuildProcessEnv::from_operator(environment))
    }

    fn with_build_env(limits: FetchLimits, build_env: BuildProcessEnv) -> Self {
        let network = crate::http::Client::from_env(&build_env);
        Self {
            acquisitions: AcquisitionGate::new(limits.concurrent),
            limits,
            network,
            build_env,
            provisioning: None,
            host_platform: crate::bottle::HostPlatform::detect(),
        }
    }

    /// The context for grip's OWN tool downloads (pixi, the BuildKit
    /// bridge helper): bound to operator env only, never the
    /// repo-declared build env this context may carry.
    pub fn provisioning(&self) -> &FetchContext {
        self.provisioning.as_deref().unwrap_or(self)
    }

    /// The platform facts bottle selection runs on (A0-01).
    pub fn host_platform(&self) -> &crate::bottle::HostPlatform {
        &self.host_platform
    }

    /// Artifact fetches may use repo-declared proxy/CA/build variables.
    /// The nested provisioning context stays bound to operator env only.
    pub fn artifacts(
        limits: FetchLimits,
        provisioning: std::sync::Arc<FetchContext>,
        build_env: BTreeMap<String, String>,
    ) -> Self {
        Self {
            provisioning: Some(provisioning),
            ..Self::with_build_env(limits, BuildProcessEnv::new(build_env))
        }
    }

    /// Apply repo build variables to a selected child, never to grip itself.
    /// Call before step-specific overrides and closure PATH composition.
    pub fn apply_build_env(&self, command: &mut Command) {
        self.build_env.apply(command);
    }

    pub(crate) fn find_fetcher(&self, name: &str) -> Option<std::path::PathBuf> {
        crate::find_fetcher_on_path(name, self.build_env.var_os("PATH"))
    }

    pub fn limits(&self) -> FetchLimits {
        self.limits
    }

    pub(crate) fn json<T: DeserializeOwned>(
        &self,
        url: &str,
        kind: crate::http::RequestKind<'_>,
    ) -> Result<T, FetchError> {
        self.network.json(url, kind)
    }

    pub(crate) fn text(
        &self,
        url: &str,
        kind: crate::http::RequestKind<'_>,
    ) -> Result<String, FetchError> {
        self.network.text(url, kind)
    }

    pub(crate) fn download(
        &self,
        url: &str,
        kind: crate::http::RequestKind<'_>,
    ) -> Result<crate::spool::Download, FetchError> {
        self.network
            .download(url, kind, self.limits.download_bytes.get())
    }

    pub(crate) fn download_hash(
        &self,
        url: &str,
        api_url: Option<&str>,
    ) -> Result<DownloadHash, FetchError> {
        self.network
            .payload_hash(url, api_url, self.limits.download_bytes.get())
    }

    pub fn resolve_latest(
        &self,
        repo: &str,
        pattern: &str,
        base: Option<&str>,
        version: Option<&str>,
    ) -> Result<crate::ResolvedRelease, crate::resolve::ResolveError> {
        crate::resolve::resolve_latest(self, repo, pattern, base, version)
    }

    pub fn resolve_brew(
        &self,
        formula: &str,
    ) -> Result<crate::ResolvedRelease, crate::resolve::ResolveError> {
        crate::resolve::resolve_brew(self, formula)
    }

    pub fn resolve_self_release(&self) -> Result<crate::SelfRelease, crate::resolve::ResolveError> {
        crate::resolve::resolve_self_release(self)
    }

    pub fn resolve_plugin_release(
        &self,
        repo: &str,
        executable: &str,
        tag: Option<&str>,
    ) -> Result<crate::resolve::PluginRelease, crate::resolve::ResolveError> {
        crate::resolve::resolve_plugin_release(self, repo, executable, tag)
    }

    pub fn fetch(
        &self,
        spec: &FetchSpec,
        dest: &Path,
        locked: Option<&serde_json::Value>,
    ) -> Result<FetchOutcome, FetchError> {
        // Provisioning can itself acquire an archive. Do not hold a permit
        // while recursively provisioning pixi, including when the cap is one.
        let pixi_executable = if matches!(spec, FetchSpec::Pixi { .. }) {
            Some(pixi::ensure(self.provisioning())?)
        } else {
            None
        };
        let _permit = self.acquisitions.acquire();
        std::fs::create_dir_all(dest)?;
        if !std::fs::symlink_metadata(dest)?.is_dir() {
            return Err(FetchError::UnsafeArchive {
                entry: dest.into(),
                reason: "payload destination is not a real directory",
            });
        }
        let outcome = match spec {
            FetchSpec::File { path } => file::fetch(self, path, dest)?,
            FetchSpec::Tarball {
                url,
                sha256,
                api_url,
            } => tarball::fetch(
                self,
                &crate::expand_platform(url),
                sha256.as_deref(),
                api_url.as_deref(),
                dest,
            )?,
            FetchSpec::Brew {
                formula,
                version,
                sha256,
            } => brew::fetch(
                self,
                formula,
                version.as_deref(),
                sha256.as_deref(),
                dest,
                locked,
            )?,
            FetchSpec::Pixi {
                package,
                version,
                sha256,
            } => {
                let pinned_version = locked
                    .and_then(|pin| pin.get("version"))
                    .and_then(|v| v.as_str())
                    .or(version.as_deref());
                let outcome = pixi::fetch(
                    self,
                    pixi_executable.as_deref().expect("pixi prepared"),
                    package,
                    pinned_version,
                    dest,
                )?;
                check_hash(package, sha256.as_deref(), &outcome.identity)?;
                outcome
            }
            FetchSpec::Git {
                url,
                rev: Some(revision),
            } => git::fetch(self, url, revision, dest)?,
            FetchSpec::Git { rev: None, .. } | FetchSpec::GithubRelease { .. } => {
                return Err(FetchError::Unsupported(
                    "source needs core resolution before acquisition".into(),
                ));
            }
            FetchSpec::Plugin { name, args } => {
                let metadata = plugin::fetch(self, name, args, dest, locked, self.limits)?;
                FetchOutcome {
                    identity: FetchIdentity::Tree(metadata.tree),
                    url: metadata.url,
                    version: metadata.version,
                }
            }
        };
        if matches!(&outcome.identity, FetchIdentity::Download(_)) {
            archive::validate_tree(dest, self.limits)?;
        }
        Ok(outcome)
    }

    /// One immutable artifact as RAW verified bytes (A3): no archive
    /// interpretation — Conda package archives are retained as opaque
    /// store objects. The shared bounded transport spools and hashes the
    /// payload; a digest mismatch is a hard failure, never a warning.
    pub fn download_verified(
        &self,
        url: &str,
        sha256: &str,
    ) -> Result<crate::spool::Download, FetchError> {
        let _permit = self.acquisitions.acquire();
        let download = tarball::download(self, url, None)?;
        if sha256 != download.hash.as_str() {
            return Err(FetchError::HashMismatch {
                url: url.into(),
                expected: sha256.into(),
                actual: download.hash.into(),
            });
        }
        Ok(download)
    }

    /// Pinned bootstrap bytes with caller-bounded acquisition and transfer.
    /// Network URLs and every redirect must use HTTPS; ambient GitHub/API
    /// credentials are never attached. Explicit file:// mirrors are accepted,
    /// so callers must pass only their trusted bootstrap manifest/mirror URL.
    /// Artifact contexts delegate to their operator-only provisioning context.
    pub fn download_tool(
        &self,
        url: &str,
        sha256: &str,
        max_bytes: std::num::NonZeroU64,
        deadline: std::time::Instant,
    ) -> Result<crate::spool::Download, FetchError> {
        if let Some(provisioning) = &self.provisioning {
            return provisioning.download_tool(url, sha256, max_bytes, deadline);
        }
        let operation_deadline = std::time::Instant::now()
            .checked_add(crate::http::OPERATION_TIMEOUT)
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "tool operation timeout exceeds Instant range",
                )
            })?;
        let deadline = deadline.min(operation_deadline);
        let expected = DownloadHash::parse(sha256)?;
        let _permit = self.acquisitions.acquire_until(deadline)?;
        let limit = max_bytes.get().min(self.limits.download_bytes.get());
        let parsed = url::Url::parse(url).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid tool acquisition URL",
            )
        })?;
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "tool acquisition URLs cannot carry credentials",
            )
            .into());
        }
        let download = match parsed.scheme() {
            "file" => {
                let path = parsed.to_file_path().map_err(|_| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "tool mirror must name an absolute local file",
                    )
                })?;
                crate::spool::download(
                    crate::spool::DeadlineReader {
                        reader: tarball::regular_file(&path)?,
                        deadline,
                    },
                    limit,
                )?
            }
            "https" => {
                self.network
                    .download(url, crate::http::RequestKind::Tool { deadline }, limit)?
            }
            _ => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "tool acquisition requires HTTPS or an explicit local file mirror",
                )
                .into());
            }
        };
        if std::time::Instant::now() >= deadline {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "tool acquisition deadline expired",
            )
            .into());
        }
        if expected != download.hash {
            return Err(FetchError::HashMismatch {
                url: crate::http::safe_location(url).into_owned(),
                expected: expected.into(),
                actual: download.hash.into(),
            });
        }
        Ok(download)
    }

    pub fn payload_hash(&self, spec: &FetchSpec) -> Result<Option<FetchIdentity>, FetchError> {
        let _permit = self.acquisitions.acquire();
        match spec {
            FetchSpec::Tarball {
                sha256: Some(hash), ..
            }
            | FetchSpec::GithubRelease {
                sha256: Some(hash), ..
            } => Ok(Some(FetchIdentity::Download(DownloadHash::parse(hash)?))),
            FetchSpec::Tarball { url, api_url, .. } => Ok(Some(FetchIdentity::Download(
                tarball::payload_hash(self, &crate::expand_platform(url), api_url.as_deref())?,
            ))),
            FetchSpec::File { path } => file::payload_hash(self, path).map(Some),
            FetchSpec::Brew { formula, .. } => self
                .resolve_brew(formula)
                .map_err(FetchError::from)?
                .sha256
                .map(|hash| {
                    DownloadHash::parse(&hash)
                        .map(FetchIdentity::Download)
                        .map_err(FetchError::from)
                })
                .transpose(),
            _ => Ok(None),
        }
    }
}

pub(crate) fn check_hash(
    source: &str,
    expected: Option<&str>,
    actual: &FetchIdentity,
) -> Result<(), FetchError> {
    if let Some(expected) = expected
        && expected != actual.as_str()
    {
        return Err(FetchError::HashMismatch {
            url: source.into(),
            expected: expected.into(),
            actual: actual.as_str().into(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tool_tests {
    use super::*;
    use std::{
        io::{Read, Seek, Write},
        num::{NonZeroU64, NonZeroUsize},
        time::{Duration, Instant},
    };
    const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    fn mirror() -> (tempfile::NamedTempFile, String) {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(b"abc").unwrap();
        let url = url::Url::from_file_path(file.path()).unwrap().to_string();
        (file, url)
    }
    fn context(limit: u64) -> FetchContext {
        let operator = gripsack_process::OperatorEnvironment::admit([]).unwrap();
        FetchContext::from_operator(
            FetchLimits {
                download_bytes: NonZeroU64::new(limit).unwrap(),
                concurrent: NonZeroUsize::new(1).unwrap(),
                ..FetchLimits::default()
            },
            &operator,
        )
    }
    #[test]
    fn verified_tool_mirror_preserves_bytes_and_enforces_both_caps() {
        let (_file, url) = mirror();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut download = context(3)
            .download_tool(&url, ABC, NonZeroU64::new(3).unwrap(), deadline)
            .unwrap();
        download.file.rewind().unwrap();
        let mut bytes = Vec::new();
        download.file.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"abc");
        for (generic, requested) in [(3, 2), (2, 3)] {
            assert!(matches!(
                context(generic).download_tool(
                    &url,
                    ABC,
                    NonZeroU64::new(requested).unwrap(),
                    deadline
                ),
                Err(FetchError::PayloadTooLarge { limit: 2, .. })
            ));
        }
        assert!(matches!(
            context(3).download_tool(&url, &"0".repeat(64), NonZeroU64::new(3).unwrap(), deadline),
            Err(FetchError::HashMismatch { .. })
        ));
    }
    #[test]
    fn queued_tool_acquisition_expires_without_reading_the_mirror() {
        let context = context(3);
        let held = context.acquisitions.acquire();
        let deadline = Instant::now() + Duration::from_millis(10);
        let result = context.download_tool(
            "file:///does-not-exist",
            ABC,
            NonZeroU64::new(3).unwrap(),
            deadline,
        );
        drop(held);
        assert!(
            matches!(result, Err(FetchError::Io(error)) if error.kind() == std::io::ErrorKind::TimedOut)
        );
    }
    #[test]
    fn tool_urls_reject_cleartext_and_embedded_credentials_without_connecting() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        for url in [
            format!("http://{}/tool", listener.local_addr().unwrap()),
            format!(
                "https://secret:password@{}/tool",
                listener.local_addr().unwrap()
            ),
        ] {
            let error = context(3)
                .download_tool(
                    &url,
                    ABC,
                    NonZeroU64::new(3).unwrap(),
                    Instant::now() + Duration::from_secs(5),
                )
                .err()
                .expect("unsafe tool URL was admitted");
            assert!(
                matches!(&error, FetchError::Io(error) if error.kind() == std::io::ErrorKind::InvalidInput)
            );
            assert!(!error.to_string().contains("secret"));
            assert!(!error.to_string().contains("password"));
        }
        assert!(
            matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
        );
    }
    #[test]
    fn operator_snapshot_does_not_inherit_missing_ambient_keys() {
        let context = context(3);
        assert_eq!(context.build_env.var_os("HOME"), None);
        assert_eq!(context.build_env.var_os("SSL_CERT_FILE"), None);
        assert_eq!(context.build_env.var_os("HTTPS_PROXY"), None);
    }
}
