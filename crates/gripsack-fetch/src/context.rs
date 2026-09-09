//! One acquisition context per command/environment phase.

use crate::fetch::{archive, brew, file, git, pixi, plugin, tarball};
use crate::limits::AcquisitionGate;
use crate::{DownloadHash, FetchError, FetchIdentity, FetchLimits, FetchOutcome};
use gripsack_ir::FetchSpec;
use serde::de::DeserializeOwned;
use std::path::Path;

pub struct FetchContext {
    limits: FetchLimits,
    network: crate::http::Client,
    acquisitions: AcquisitionGate,
    provisioning: Option<std::sync::Arc<FetchContext>>,
}

impl Default for FetchContext {
    fn default() -> Self {
        Self::new(FetchLimits::default())
    }
}

impl FetchContext {
    pub fn new(limits: FetchLimits) -> Self {
        Self {
            acquisitions: AcquisitionGate::new(limits.concurrent),
            limits,
            network: crate::http::Client::from_env(),
            provisioning: None,
        }
    }

    /// Capture artifact policy after repo env injection, retaining the earlier
    /// trusted-tool policy for lazy runtime provisioning.
    pub fn artifacts(limits: FetchLimits, provisioning: std::sync::Arc<FetchContext>) -> Self {
        Self {
            provisioning: Some(provisioning),
            ..Self::new(limits)
        }
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
            Some(pixi::ensure(self.provisioning.as_deref().unwrap_or(self))?)
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
                let metadata = plugin::fetch(name, args, dest, locked, self.limits)?;
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
