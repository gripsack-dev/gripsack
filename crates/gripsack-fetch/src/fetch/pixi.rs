//! Pixi installs into a stable private prefix; only the harvested payload is
//! hashed. The primary package's actual version comes from conda metadata.

use crate::{FetchContext, FetchError, FetchIdentity, FetchOutcome};
use std::path::{Path, PathBuf};

pub(crate) fn ensure(context: &FetchContext) -> Result<PathBuf, FetchError> {
    let home = gripsack_store::gripsack_home();
    let tools = home.join("tools");
    let dir = tools.join(format!("pixi-{}", crate::host::PIXI_RELEASE.version));
    let executable = dir.join("pixi");
    if executable.is_file() {
        return Ok(executable);
    }
    let _lock = gripsack_fs::FlockGuard::acquire(&home.join("locks"), "pixi")?;
    if executable.is_file() {
        return Ok(executable);
    }
    let (url, sha) = crate::host::resolve(&crate::host::PIXI_RELEASE)?;
    std::fs::create_dir_all(&tools)?;
    let staging = tempfile::Builder::new()
        .prefix(".pixi-")
        .tempdir_in(&tools)?;
    context.fetch(
        &gripsack_ir::FetchSpec::Tarball {
            url,
            sha256: Some(sha.into()),
            api_url: None,
        },
        staging.path(),
        None,
    )?;
    let source = staging.path().join("pixi");
    if !std::fs::symlink_metadata(&source)?.is_file() {
        return Err(failure(
            "pixi",
            "bundled archive did not contain a regular pixi executable",
        ));
    }
    std::fs::create_dir_all(&dir)?;
    std::fs::rename(source, &executable)?;
    Ok(executable)
}

pub(crate) fn fetch(
    context: &FetchContext,
    executable: &Path,
    package: &str,
    version: Option<&str>,
    dest: &Path,
) -> Result<FetchOutcome, FetchError> {
    let unqualified = package.rsplit("::").next().unwrap_or(package);
    let name = unqualified
        .split(|ch: char| ch.is_whitespace() || "[=<>!~".contains(ch))
        .next()
        .unwrap_or("");
    if name.is_empty()
        || matches!(name, "." | "..")
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
    {
        return Err(failure(
            package,
            "package specification has no valid primary package name",
        ));
    }
    let spec = match version {
        Some(version) => format!("{package}=={version}"),
        None => package.to_owned(),
    };
    let home = gripsack_store::gripsack_home();
    // Global install mutates one manifest as well as the selected prefix.
    // Serialize it even if the author omitted a pixi-lock resource.
    let _lock = gripsack_fs::FlockGuard::acquire(&home.join("locks"), "pixi-install")?;
    let pixi_home = home.join("tools/pixi");
    let mut command = std::process::Command::new(executable);
    command
        .args([
            "global",
            "install",
            "--force-reinstall",
            "--no-shortcuts",
            "--concurrent-downloads",
            "1",
            "--concurrent-solves",
            "1",
            "--",
            &spec,
        ])
        .env("PIXI_HOME", &pixi_home);
    let outcome = gripsack_process::run(&mut command, &[], Default::default(), |_| {
        gripsack_process::Control::Continue
    })?;
    if !matches!(outcome.reason, gripsack_process::StopReason::Exited)
        || !outcome.status.is_some_and(|status| status.success())
    {
        return Err(failure(
            package,
            format!(
                "pixi install stopped ({:?}, {:?}): {}",
                outcome.reason,
                outcome.status,
                String::from_utf8_lossy(&outcome.stderr)
            ),
        ));
    }
    let prefix = pixi_home.join("envs").join(name);
    let resolved = installed_version(&prefix, name, context.limits())?;
    if let Some(expected) = version
        && expected != resolved
    {
        return Err(failure(
            package,
            format!("installed version {resolved} differs from pinned version {expected}"),
        ));
    }
    super::archive::copy_tree_filtered(&prefix, dest, &["conda-meta"], context.limits())?;
    super::archive::validate_tree(dest, context.limits())?;
    Ok(FetchOutcome {
        identity: FetchIdentity::Tree(gripsack_store::canonical_tree_hash(dest)?),
        url: None,
        version: Some(resolved),
    })
}

fn failure(package: &str, reason: impl Into<String>) -> FetchError {
    FetchError::Http {
        url: package.into(),
        reason: reason.into(),
    }
}

fn installed_version(
    prefix: &Path,
    package: &str,
    limits: crate::FetchLimits,
) -> Result<String, FetchError> {
    #[derive(serde::Deserialize)]
    struct Record {
        name: String,
        version: String,
    }
    let mut version = None;
    for (index, entry) in std::fs::read_dir(prefix.join("conda-meta"))?.enumerate() {
        if index >= limits.archive_entries.get() {
            return Err(FetchError::TooManyEntries {
                limit: limits.archive_entries.get(),
            });
        }
        let entry = entry?;
        if entry
            .path()
            .extension()
            .is_none_or(|extension| extension != "json")
        {
            continue;
        }
        let record: Record = serde_json::from_reader(crate::spool::Limited::new(
            super::tarball::regular_file(&entry.path())?,
            8 * 1024 * 1024,
            "conda metadata",
        ))
        .map_err(|error| failure(package, format!("invalid package metadata: {error}")))?;
        if record.name.eq_ignore_ascii_case(package) {
            if record.version.is_empty() || version.is_some() {
                return Err(failure(
                    package,
                    "missing or ambiguous installed primary-package version",
                ));
            }
            version = Some(record.version);
        }
    }
    version.ok_or_else(|| failure(package, "installed primary-package metadata is missing"))
}
