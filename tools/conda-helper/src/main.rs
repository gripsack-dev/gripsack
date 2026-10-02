//! `gripsack-conda` helper: the out-of-process Rattler solver and installer.
//!
//! Speaks one-shot protocol v2 (one versioned length-prefixed request and
//! response over stdio), implemented by `gripsack_conda::protocol`.
//!
//! * `resolve` — queries channel repodata over anonymous rustls HTTPS and
//!   solves via `rattler_solve` (resolvo backend, strict channel priority)
//!   with the core-measured virtual packages as the virtual set.
//! * `import_pixi` — validates captured Pixi manifest/lock satisfaction with
//!   canonical upstream selection and MatchSpec semantics, without network.
//! * `materialize` — frozen install from retained archives: rehashes every
//!   archive, extracts via `rattler_package_streaming`, refuses packages with
//!   pre/post-link or activate/deactivate scripts, links with Rattler's
//!   `link_package` patched for the final prefix (copy mode, clobber = hard
//!   error), and writes normalized conda-meta receipts. NO network access.
//!
//! Helper success is advisory: the core re-validates independently.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::str::FromStr;

use gripsack_conda::protocol::{
    self, ArchiveRef, ErrorResponse, ImportedResponse, MaterializeRequest, MaterializedPackage,
    MaterializedResponse, Request, ResolveRequest, ResolvedResponse, Response,
};
use gripsack_ir::workspace_v6::lock::{
    BytecodePolicy, ChannelPriority, LockedCondaEnvironment, LockedCondaPackage, LockedNoArch,
    LockedRunExports, MaterializerPolicy, ReceiptPolicy,
};
use rattler::install::{
    AppleCodeSignBehavior, ClobberMode, InstallDriver, InstallOptions, PythonInfo, link_package,
};
use rattler_conda_types::package::{IndexJson, LinkJson, NoArchLinks, PackageFile, PathsJson};
use rattler_conda_types::prefix::Prefix;
use rattler_conda_types::{
    Channel, GenericVirtualPackage, MatchSpec, NoArchKind, PackageName, ParseStrictness,
    RepoDataRecord, Subdir, Version,
};
mod pixi_import;
use pixi_import::import_pixi;
use rattler_networking::LazyClient;
use rattler_repodata_gateway::Gateway;
use rattler_solve::{
    ChannelPriority as SolveChannelPriority, RepoDataIter, SolveStrategy, SolverImpl, SolverTask,
};
use sha2::{Digest, Sha256};

fn main() -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("gripsack-conda: failed to start runtime: {error}");
            return ExitCode::FAILURE;
        }
    };
    runtime.block_on(serve())
}

/// One request failure: a stable machine-readable code plus a message.
struct Failure {
    code: &'static str,
    message: String,
}

fn fail(code: &'static str, message: String) -> Failure {
    Failure { code, message }
}

type HResult<T> = Result<T, Failure>;

async fn serve() -> ExitCode {
    let mut input = io::BufReader::new(io::stdin());
    let mut output = io::BufWriter::new(io::stdout());
    let request = match protocol::read_request_frame(&mut input) {
        Ok(Some(request)) => request,
        Ok(None) => {
            send_error(
                &mut output,
                None,
                "protocol",
                "missing request frame".into(),
            );
            return ExitCode::FAILURE;
        }
        Err(error) => {
            send_error(&mut output, None, "protocol", error.to_string());
            return ExitCode::FAILURE;
        }
    };
    let attempt = request.attempt();
    // Admit the complete one-shot input before any resolver or installer effect.
    let mut extra = [0];
    match input.read(&mut extra) {
        Ok(0) => {}
        Ok(_) => {
            send_error(
                &mut output,
                Some(attempt),
                "protocol",
                "bytes follow the request frame".into(),
            );
            return ExitCode::FAILURE;
        }
        Err(error) => {
            send_error(&mut output, Some(attempt), "protocol", error.to_string());
            return ExitCode::FAILURE;
        }
    }
    let response = match request {
        Request::Resolve(request) => resolve(request).await.map(|environment| {
            Response::Resolved(ResolvedResponse {
                attempt,
                environment,
            })
        }),
        Request::ImportPixi(request) => import_pixi(request).map(|environment| {
            Response::Imported(ImportedResponse {
                attempt,
                environment,
            })
        }),
        Request::Materialize(request) => materialize(request).await.map(Response::Materialized),
    }
    .unwrap_or_else(|failure| {
        Response::Error(ErrorResponse {
            attempt: Some(attempt),
            code: failure.code.to_string(),
            message: failure.message,
        })
    });
    match protocol::write_response_frame(&mut output, &response) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

fn send_error(output: &mut impl Write, attempt: Option<u64>, code: &str, message: String) {
    let _ = protocol::write_response_frame(
        output,
        &Response::Error(ErrorResponse {
            attempt,
            code: code.to_string(),
            message,
        }),
    );
}

// ---------------------------------------------------------------------------
// resolve
// ---------------------------------------------------------------------------

/// Anonymous HTTPS only: any URL carrying userinfo credentials is a strict
/// error.
fn reject_credentials(url: &reqwest::Url, what: &str) -> HResult<()> {
    if !url.username().is_empty() || url.password().is_some() {
        return Err(fail(
            "credentials_rejected",
            format!("{what} carries userinfo credentials: {}", redact(url)),
        ));
    }
    Ok(())
}

fn redact(url: &reqwest::Url) -> String {
    let mut redacted = url.clone();
    let _ = redacted.set_username("");
    let _ = redacted.set_password(None);
    redacted.to_string()
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

async fn resolve(request: ResolveRequest) -> HResult<LockedCondaEnvironment> {
    let mut channels = Vec::with_capacity(request.channels.len());
    let mut canonical_channels = Vec::with_capacity(request.channels.len());
    for raw in &request.channels {
        // Contract C1: bare names canonicalize to conda.anaconda.org;
        // absolute https:// URLs pass through; everything else refuses.
        let canonical = gripsack_conda::channels::canonicalize_channel(raw)
            .map_err(|error| fail("invalid_channel", error.to_string()))?;
        let url = reqwest::Url::parse(&canonical).map_err(|error| {
            fail(
                "invalid_channel",
                format!("channel '{raw}' canonicalized to an unparseable URL: {error}"),
            )
        })?;
        reject_credentials(&url, "channel")?;
        if url.scheme() != "https" {
            return Err(fail(
                "invalid_channel",
                format!(
                    "channel '{raw}': anonymous HTTPS only, got scheme '{}'",
                    url.scheme()
                ),
            ));
        }
        let channel = Channel::from_url(url);
        // IR canonical form carries no trailing slash.
        canonical_channels.push(channel.canonical_name().trim_end_matches('/').to_string());
        channels.push(channel);
    }
    if channels.is_empty() {
        return Err(fail("invalid_channel", "no channels supplied".into()));
    }

    let subdir = Subdir::from_str(&request.platform).map_err(|error| {
        fail(
            "invalid_platform",
            format!("platform '{}': {error}", request.platform),
        )
    })?;

    let mut virtual_packages = Vec::with_capacity(request.virtual_packages.len());
    for virtual_package in &request.virtual_packages {
        let name = PackageName::from_str(&virtual_package.name).map_err(|error| {
            fail(
                "invalid_virtual_package",
                format!("virtual package name '{}': {error}", virtual_package.name),
            )
        })?;
        let version = Version::from_str(&virtual_package.version).map_err(|error| {
            fail(
                "invalid_virtual_package",
                format!(
                    "virtual package '{}' version '{}': {error}",
                    virtual_package.name, virtual_package.version
                ),
            )
        })?;
        virtual_packages.push(GenericVirtualPackage {
            name,
            version,
            build_string: virtual_package.build.clone(),
        });
    }

    let mut specs = Vec::with_capacity(request.packages.len());
    for (name, constraint) in &request.packages {
        let source = if constraint.trim().is_empty() {
            name.clone()
        } else {
            format!("{name} {constraint}")
        };
        specs.push(
            MatchSpec::from_str(&source, ParseStrictness::Strict).map_err(|error| {
                fail(
                    "invalid_matchspec",
                    format!("package spec '{source}': {error}"),
                )
            })?,
        );
    }

    let client = reqwest::Client::builder()
        .https_only(true)
        .user_agent(concat!("gripsack-conda/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| fail("repodata", format!("failed to build HTTP client: {error}")))?;
    let cache_dir =
        std::env::temp_dir().join(format!("gripsack-conda-repodata-{}", std::process::id()));
    let gateway = Gateway::builder()
        .with_client(LazyClient::from(client))
        .with_cache_dir(&cache_dir)
        .finish();

    // The solve needs candidates for the whole transitive dependency graph,
    // so the query recursively discovers dependencies.
    let repodata = gateway
        .query(channels, [subdir, Subdir::NoArch], specs.clone())
        .recursive(true)
        .await
        .map_err(|error| fail("repodata", format!("repodata query failed: {error}")))?;

    let available: Vec<RepoDataIter<_>> = repodata.repodata.iter().map(RepoDataIter).collect();
    let task = SolverTask {
        available_packages: available,
        locked_packages: Vec::new(),
        pinned_packages: Vec::new(),
        virtual_packages,
        specs,
        constraints: Vec::new(),
        timeout: None,
        channel_priority: SolveChannelPriority::Strict,
        exclude_newer: None,
        strategy: SolveStrategy::default(),
        dependency_overrides: Vec::new(),
        excluded_candidates: HashMap::new(),
        cancellation_token: None,
    };
    let mut solver = rattler_solve::resolvo::Solver;
    let solution = solver
        .solve(task)
        .map_err(|error| fail("solve_failed", error.to_string()))?;

    let mut packages = solution
        .records
        .iter()
        .map(|record| normalize_record(record, &canonical_channels))
        .collect::<HResult<Vec<_>>>()?;
    // Canonical order: name, then the full record identity.
    packages.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.version.cmp(&b.version))
            .then_with(|| a.build.cmp(&b.build))
            .then_with(|| a.subdir.cmp(&b.subdir))
            .then_with(|| a.sha256.cmp(&b.sha256))
    });

    Ok(LockedCondaEnvironment {
        platform: request.platform,
        channels: canonical_channels,
        channel_priority: ChannelPriority::Strict,
        system_requirements: Default::default(),
        virtual_packages: request.virtual_packages,
        packages,
        materializer: MaterializerPolicy {
            bytecode: BytecodePolicy::Suppress,
            receipt: ReceiptPolicy::NormalizedCondaMeta,
        },
    })
}

/// Normalizes one Rattler record into a [`LockedCondaPackage`], preserving
/// every install-critical field. `sha256` is mandatory: MD5-only records are
/// a strict-import error.
fn normalize_record(
    record: &RepoDataRecord,
    canonical_channels: &[String],
) -> HResult<LockedCondaPackage> {
    let package_record = &record.package_record;
    let name = package_record.name.as_normalized().to_string();
    reject_credentials(&record.url, &format!("package '{name}' artifact URL"))?;
    let sha256 = package_record.sha256.as_ref().map(|hash| hex_lower(hash)).ok_or_else(|| {
        fail(
            "md5_only",
            format!(
                "package '{name}' ({}) carries no sha256; MD5-only records are a strict-import error",
                record.url
            ),
        )
    })?;
    let url = record.url.to_string();
    let channel = canonical_channels
        .iter()
        .find(|channel| url.starts_with(&format!("{channel}/")))
        .cloned()
        .or_else(|| channel_base_from_url(&url))
        .ok_or_else(|| {
            fail(
                "invalid_record",
                format!("package '{name}': cannot derive channel base URL from {url}"),
            )
        })?;
    let timestamp = package_record
        .timestamp
        .map(|timestamp| {
            u64::try_from(timestamp.jiff_timestamp().as_millisecond()).map_err(|_| {
                fail(
                    "invalid_record",
                    format!("package '{name}': negative repodata timestamp"),
                )
            })
        })
        .transpose()?;
    let indexed_timestamp = package_record
        .indexed_timestamp
        .map(|timestamp| {
            u64::try_from(timestamp.jiff_timestamp().as_millisecond()).map_err(|_| {
                fail(
                    "invalid_record",
                    format!("package '{name}': negative indexing timestamp"),
                )
            })
        })
        .transpose()?;
    Ok(LockedCondaPackage {
        name,
        version: package_record.version.to_string(),
        build: package_record.build.clone(),
        build_number: package_record.build_number,
        subdir: package_record.subdir.clone(),
        channel,
        url,
        sha256,
        size: package_record.size,
        timestamp,
        indexed_timestamp,
        attestations_sha256: package_record
            .attestations_sha256
            .as_ref()
            .map(|hash| hex_lower(hash)),
        md5: package_record.md5.as_ref().map(|hash| hex_lower(hash)),
        legacy_bz2_md5: package_record
            .legacy_bz2_md5
            .as_ref()
            .map(|hash| hex_lower(hash)),
        legacy_bz2_size: package_record.legacy_bz2_size,
        arch: package_record.arch.clone(),
        platform: package_record.platform.clone(),
        noarch: match package_record.noarch.kind() {
            None => LockedNoArch::None,
            Some(NoArchKind::Generic) => LockedNoArch::Generic,
            Some(NoArchKind::Python) => LockedNoArch::Python,
        },
        license: package_record.license.clone(),
        license_family: package_record.license_family.clone(),
        depends: package_record.depends.clone(),
        constrains: package_record.constrains.clone(),
        extra_depends: package_record.extra_depends.clone(),
        flags: package_record
            .flags
            .iter()
            .map(ToString::to_string)
            .collect(),
        python_site_packages_path: package_record.python_site_packages_path.clone(),
        run_exports: package_record
            .run_exports
            .as_ref()
            .map(|exports| LockedRunExports {
                weak: exports.weak.clone(),
                strong: exports.strong.clone(),
                noarch: exports.noarch.clone(),
                weak_constrains: exports.weak_constrains.clone(),
                strong_constrains: exports.strong_constrains.clone(),
            }),
        purls: package_record
            .purls
            .as_ref()
            .map(|purls| purls.iter().map(ToString::to_string).collect()),
        track_features: package_record.track_features.clone(),
        features: package_record.features.clone(),
    })
}

/// Fallback channel derivation: strip the trailing `<subdir>/<filename>` from
/// the artifact URL.
fn channel_base_from_url(url: &str) -> Option<String> {
    let mut parts = url.rsplitn(3, '/');
    parts.next()?; // filename
    parts.next()?; // subdir
    let base = parts.next()?;
    if base.is_empty() {
        None
    } else {
        Some(base.to_string())
    }
}

// ---------------------------------------------------------------------------
// materialize (frozen: never solves, NO network I/O)
// ---------------------------------------------------------------------------

/// Removes the helper's extraction scratch tree on every exit path.
struct ScratchDir(PathBuf);

impl ScratchDir {
    fn create(attempt: u64) -> HResult<Self> {
        let path = std::env::temp_dir().join(format!(
            "gripsack-conda-extract-{}-{attempt}",
            std::process::id()
        ));
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .map_err(|error| {
                fail(
                    "io",
                    format!(
                        "failed to exclusively create private scratch dir {}: {error}",
                        path.display()
                    ),
                )
            })?;
        Ok(ScratchDir(path))
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn validate_sha256_hex(sha256: &str, what: &str) -> HResult<String> {
    let normalized = sha256.to_ascii_lowercase();
    if normalized.len() != 64 || !normalized.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(fail(
            "invalid_request",
            format!("{what} is not 64 lowercase hex: '{sha256}'"),
        ));
    }
    Ok(normalized)
}

fn rehash(path: &Path) -> HResult<String> {
    let mut file = std::fs::File::open(path).map_err(|error| {
        fail(
            "io",
            format!("failed to open archive {}: {error}", path.display()),
        )
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            fail(
                "io",
                format!("failed to read archive {}: {error}", path.display()),
            )
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex_lower(&hasher.finalize()))
}

/// Refuses packages whose archive contains pre/post-link scripts or
/// activate/deactivate scripts. The error always names the exact package.
fn refuse_package_scripts(package_dir: &Path, name: &str) -> HResult<()> {
    let mut suspects: Vec<PathBuf> = Vec::new();
    for script in [
        "pre-link.sh",
        "post-link.sh",
        "pre-unlink.sh",
        "pre-link.bat",
        "post-link.bat",
        "pre-unlink.bat",
    ] {
        suspects.push(package_dir.join("info/recipe").join(script));
    }
    for script in ["pre-link.sh", "post-link.sh", "pre-unlink.sh"] {
        suspects.push(package_dir.join(format!("bin/.{name}-{script}")));
    }
    for script in ["pre-link.bat", "post-link.bat", "pre-unlink.bat"] {
        suspects.push(package_dir.join(format!("Scripts/.{name}-{script}")));
    }
    for directory in [
        "etc/conda/activate.d",
        "etc/conda/deactivate.d",
        "etc/activate.d",
        "etc/deactivate.d",
    ] {
        suspects.push(package_dir.join(directory));
    }
    for suspect in suspects {
        if suspect.symlink_metadata().is_ok() {
            let relative = suspect.strip_prefix(package_dir).unwrap_or(&suspect);
            return Err(fail(
                "link_script",
                format!(
                    "package '{name}' ships link/activation script '{}'; refusing to materialize it",
                    relative.display()
                ),
            ));
        }
    }
    Ok(())
}

/// One extracted, verified archive ready to link.
struct PreparedPackage<'a> {
    directory: PathBuf,
    index: IndexJson,
    record: &'a LockedCondaPackage,
}

async fn materialize(request: MaterializeRequest<'_>) -> HResult<MaterializedResponse> {
    let closure = request.closure.into_owned();
    // v1 admits exactly one policy; destructuring proves it (any new policy
    // variant is a compile error here, never a silent default).
    let MaterializerPolicy {
        bytecode: BytecodePolicy::Suppress,
        receipt: ReceiptPolicy::NormalizedCondaMeta,
    } = closure.materializer;

    let lock_digest = validate_sha256_hex(&request.lock_digest, "lock_digest")?;
    if gripsack_ir::workspace_v6::identity::conda_closure_digest(&closure).to_string()
        != lock_digest
    {
        return Err(fail(
            "invalid_request",
            "frozen closure digest differs from its records".into(),
        ));
    }
    let mut archives = BTreeMap::new();
    for archive in &request.archives {
        if archives.insert(archive.sha256.as_str(), archive).is_some() {
            return Err(fail("invalid_request", "duplicate retained archive".into()));
        }
    }
    let expected_archives: BTreeSet<_> = closure
        .packages
        .iter()
        .map(|record| record.sha256.as_str())
        .collect();
    if archives.keys().copied().collect::<BTreeSet<_>>() != expected_archives {
        return Err(fail(
            "invalid_request",
            "retained archive set differs from the frozen closure".into(),
        ));
    }
    let final_prefix = PathBuf::from(&request.final_prefix);
    if !final_prefix.is_absolute() {
        return Err(fail(
            "invalid_request",
            format!("final_prefix is not absolute: '{}'", request.final_prefix),
        ));
    }
    let staging_dir = PathBuf::from(&request.staging_dir);
    if !staging_dir.is_absolute() {
        return Err(fail(
            "invalid_request",
            format!("staging_dir is not absolute: '{}'", request.staging_dir),
        ));
    }
    let staging_metadata = std::fs::metadata(&staging_dir).map_err(|error| {
        fail(
            "invalid_staging",
            format!("staging dir {}: {error}", staging_dir.display()),
        )
    })?;
    if !staging_metadata.is_dir() {
        return Err(fail(
            "invalid_staging",
            format!("staging dir {} is not a directory", staging_dir.display()),
        ));
    }
    if std::fs::read_dir(&staging_dir)
        .map_err(|error| {
            fail(
                "invalid_staging",
                format!("staging dir unreadable: {error}"),
            )
        })?
        .next()
        .is_some()
    {
        return Err(fail(
            "invalid_staging",
            format!("staging dir {} is not empty", staging_dir.display()),
        ));
    }
    let subdir = Subdir::from_str(&closure.platform).map_err(|error| {
        fail(
            "invalid_platform",
            format!("platform '{}': {error}", closure.platform),
        )
    })?;

    // Phase 1: rehash, extract, and validate every archive.
    let scratch = ScratchDir::create(request.attempt)?;
    let mut prepared = Vec::with_capacity(closure.packages.len());
    for (position, record) in closure.packages.iter().enumerate() {
        if record.subdir != closure.platform && record.subdir != "noarch" {
            return Err(fail(
                "invalid_platform",
                format!("package '{}' belongs to {}", record.name, record.subdir),
            ));
        }
        prepared.push(prepare_archive(
            archives[record.sha256.as_str()],
            record,
            &scratch.0,
            position,
        )?);
    }

    // Python information for noarch python entry-point generation, derived
    // from the closure's own python package.
    let python_info = prepared
        .iter()
        .find(|package| package.index.name.as_normalized() == "python")
        .map(|package| {
            PythonInfo::from_version(
                package.index.version.version(),
                package.index.python_site_packages_path.as_deref(),
                subdir,
            )
            .map_err(|error| fail("invalid_python", format!("closure python package: {error}")))
        })
        .transpose()?;

    /// BytecodePolicy::Suppress: drop every `.pyc`/`.pyo` entry from the paths
    /// list so precompiled bytecode shipped inside an archive is never linked
    /// into the prefix.
    fn suppress_bytecode(paths_json: PathsJson) -> PathsJson {
        let mut filtered = paths_json;
        filtered.paths.retain(|entry| {
            let path = entry.relative_path.to_string_lossy();
            !path.ends_with(".pyc") && !path.ends_with(".pyo")
        });
        filtered
    }

    // Phase 2: ClobberMode::Error pre-check — staging starts empty and no two
    // packages may install the same path. BytecodePolicy::Suppress: `.pyc`
    // payload entries (some archives ship precompiled bytecode) are filtered
    // out of the overridden paths.json, so no `.pyc` is ever written.
    let mut owners: HashMap<String, String> = HashMap::new();
    let mut directory_paths = BTreeSet::new();
    let mut package_paths = Vec::with_capacity(prepared.len());
    for package in &prepared {
        let name = package.index.name.as_normalized().to_string();
        let paths_json =
            PathsJson::from_package_directory_with_deprecated_fallback(&package.directory)
                .map_err(|error| {
                    fail(
                        "invalid_package",
                        format!("package '{name}': failed to read info/paths.json: {error}"),
                    )
                })?;
        let paths_json = suppress_bytecode(paths_json);
        let mut targets = Vec::new();
        for entry in &paths_json.paths {
            let target = if package.index.noarch.is_python() {
                python_info
                    .as_ref()
                    .ok_or_else(|| {
                        fail(
                            "invalid_python",
                            format!("package '{name}' requires frozen Python"),
                        )
                    })?
                    .get_python_noarch_target_path(&entry.relative_path)
                    .into_owned()
            } else {
                entry.relative_path.clone()
            };
            if entry.path_type == rattler_conda_types::package::PathType::Directory {
                directory_paths.insert(target.to_string_lossy().into_owned());
            }
            targets.push(target);
        }
        if package.index.noarch.is_python() {
            match LinkJson::from_package_directory(&package.directory) {
                Ok(link) => {
                    if link.package_metadata_version != 1 {
                        return Err(fail(
                            "invalid_package",
                            format!("package '{name}' has unsupported link metadata"),
                        ));
                    }
                    let NoArchLinks::Python(points) = link.noarch else {
                        return Err(fail(
                            "invalid_package",
                            format!("package '{name}' has non-Python link metadata"),
                        ));
                    };
                    for point in points.entry_points {
                        targets.push(PathBuf::from("bin").join(point.command));
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(fail(
                        "invalid_package",
                        format!("package '{name}' link metadata: {error}"),
                    ));
                }
            }
        }
        for target in targets {
            let relative = target
                .to_str()
                .ok_or_else(|| fail("invalid_package", "non-UTF-8 install path".into()))?;
            if relative.is_empty()
                || relative.contains('\\')
                || relative
                    .split('/')
                    .any(|part| matches!(part, "" | "." | ".."))
                || relative == "conda-meta"
                || relative.starts_with("conda-meta/")
            {
                return Err(fail(
                    "invalid_package",
                    format!("package '{name}' carries an unsafe/reserved install path"),
                ));
            }
            if let Some(previous) = owners.insert(relative.to_owned(), name.clone()) {
                return Err(fail(
                    "clobber",
                    format!(
                        "packages '{previous}' and '{name}' both install '{relative}' (ClobberMode::Error)"
                    ),
                ));
            }
        }
        package_paths.push((name, paths_json));
    }
    // Generated entry points are not protected by Rattler's clobber registry.
    // Check their relocated destinations and ancestor conflicts before any link.
    for (path, name) in &owners {
        let mut ancestor = path.as_str();
        while let Some((parent, _)) = ancestor.rsplit_once('/') {
            if !directory_paths.contains(parent) {
                if let Some(previous) = owners.get(parent) {
                    return Err(fail(
                        "clobber",
                        format!(
                            "package '{name}' path '{path}' traverses path '{parent}' owned by '{previous}'"
                        ),
                    ));
                }
            }
            ancestor = parent;
        }
    }

    // Phase 3: link every package, bytes patched for final_prefix while
    // writing under staging_dir. Copy mode only: never hardlink the shared
    // archives. The driver never executes link scripts (such packages are
    // refused above regardless) and errors on clobbers.
    let driver = InstallDriver::builder()
        .clobber_mode(ClobberMode::Error)
        .execute_link_scripts(false)
        .finish();
    let prefix = Prefix::create(&staging_dir).map_err(|error| {
        fail(
            "io",
            format!("failed to create prefix {}: {error}", staging_dir.display()),
        )
    })?;
    // `Prefix::create` writes a CACHEDIR.TAG for backup-exclusion; a frozen
    // tree carries exactly archive payload plus receipts, so drop it.
    let _ = std::fs::remove_file(staging_dir.join("CACHEDIR.TAG"));
    let mut receipts = Vec::with_capacity(prepared.len());
    for (package, (name, paths_json)) in prepared.into_iter().zip(package_paths) {
        let options = InstallOptions {
            paths_json: Some(paths_json),
            index_json: Some(package.index),
            target_prefix: Some(final_prefix.clone()),
            allow_symbolic_links: None,
            allow_hard_links: Some(false),
            allow_ref_links: Some(false),
            platform: Some(subdir),
            python_info: python_info.clone(),
            apple_codesign_behavior: AppleCodeSignBehavior::default(),
            ..InstallOptions::default()
        };
        let installed = link_package(&package.directory, &prefix, &driver, options)
            .await
            .map_err(|error| {
                fail(
                    "link_failed",
                    format!("package '{name}': link failed: {error}"),
                )
            })?;
        let installed_paths = installed
            .into_iter()
            .map(|entry| {
                entry
                    .relative_path
                    .into_os_string()
                    .into_string()
                    .map_err(|_| {
                        fail(
                            "invalid_package",
                            format!("package '{name}' has a non-UTF-8 installed path"),
                        )
                    })
            })
            .collect::<HResult<BTreeSet<_>>>()?;
        receipts.push(write_receipt(
            &staging_dir,
            package.record,
            &installed_paths,
            &request.final_prefix,
        )?);
    }

    // ReceiptPolicy::NormalizedCondaMeta is exact-set: the tree is archive
    // payload plus normalized per-package receipts and nothing else. The
    // rattler prefix machinery appends a conda-meta/history log per link;
    // it is not a per-package receipt, so drop it.
    let _ = std::fs::remove_file(staging_dir.join("conda-meta/history"));

    Ok(MaterializedResponse {
        attempt: request.attempt,
        lock_digest,
        platform: closure.platform,
        final_prefix: request.final_prefix,
        packages: receipts,
    })
}

/// The two archive formats a conda closure may carry.
enum ArchiveFormat {
    /// `.conda`: a ZIP container (`PK\x03\x04`).
    Conda,
    /// `.tar.bz2`: bzip2 stream (`BZh`).
    TarBz2,
}

/// Detects the archive format from magic bytes; the file name is irrelevant
/// (retained archives are digest-named and extensionless).
fn detect_archive_format(path: &Path) -> HResult<ArchiveFormat> {
    let mut file = std::fs::File::open(path).map_err(|error| {
        fail(
            "io",
            format!("failed to open archive {}: {error}", path.display()),
        )
    })?;
    let mut magic = [0u8; 4];
    let read = file.read(&mut magic).map_err(|error| {
        fail(
            "io",
            format!("failed to read archive {}: {error}", path.display()),
        )
    })?;
    if read == 4 && magic == *b"PK\x03\x04" {
        return Ok(ArchiveFormat::Conda);
    }
    if read >= 3 && magic[..3] == *b"BZh" {
        return Ok(ArchiveFormat::TarBz2);
    }
    Err(fail(
        "extract_failed",
        format!(
            "archive {}: unsupported package archive format",
            path.display()
        ),
    ))
}

fn prepare_archive<'a>(
    archive: &ArchiveRef,
    record: &'a LockedCondaPackage,
    scratch: &Path,
    position: usize,
) -> HResult<PreparedPackage<'a>> {
    let expected = validate_sha256_hex(&archive.sha256, "archive sha256")?;
    let archive_path = PathBuf::from(&archive.path);
    let actual = rehash(&archive_path)?;
    if actual != expected {
        return Err(fail(
            "hash_mismatch",
            format!(
                "archive {} hashes to {actual}, expected {expected}",
                archive_path.display()
            ),
        ));
    }
    let directory = scratch.join(format!("pkg-{position}"));
    std::fs::create_dir_all(&directory).map_err(|error| {
        fail(
            "io",
            format!(
                "failed to create extraction dir {}: {error}",
                directory.display()
            ),
        )
    })?;
    // The core retains archives extensionless (store objects are named by
    // digest), so the format is detected from magic bytes and dispatched to
    // the matching extraction entry point — never the extension dispatcher.
    match detect_archive_format(&archive_path)? {
        ArchiveFormat::Conda => {
            rattler_package_streaming::fs::extract_conda(&archive_path, &directory)
        }
        ArchiveFormat::TarBz2 => {
            rattler_package_streaming::fs::extract_tar_bz2(&archive_path, &directory)
        }
    }
    .map_err(|error| {
        fail(
            "extract_failed",
            format!("failed to extract {}: {error}", archive_path.display()),
        )
    })?;
    let index = IndexJson::from_package_directory(&directory).map_err(|error| {
        fail(
            "invalid_package",
            format!(
                "archive {}: failed to read info/index.json: {error}",
                archive_path.display()
            ),
        )
    })?;
    let frozen = gripsack_conda::records::package_record(record)
        .map_err(|error| fail("invalid_record", error.to_string()))?;
    if index.name != frozen.name
        || index.version != frozen.version
        || index.build != frozen.build
        || index.build_number != frozen.build_number
    {
        return Err(fail(
            "record_mismatch",
            format!(
                "package '{}' archive index differs from its frozen identity",
                record.name
            ),
        ));
    }
    refuse_package_scripts(&directory, &record.name)?;
    // Repodata patches are frozen authority. The archive's original dependency
    // metadata and Python layout must not silently replace the solved record.
    let index = IndexJson {
        arch: frozen.arch,
        build: frozen.build,
        build_number: frozen.build_number,
        constrains: frozen.constrains,
        depends: frozen.depends,
        extra_depends: frozen.extra_depends,
        features: frozen.features,
        flags: frozen.flags,
        license: frozen.license,
        license_family: frozen.license_family,
        name: frozen.name,
        noarch: frozen.noarch,
        platform: frozen.platform,
        purls: frozen.purls,
        python_site_packages_path: frozen.python_site_packages_path,
        repodata_revision: index.repodata_revision,
        subdir: Some(frozen.subdir),
        timestamp: frozen.timestamp,
        track_features: frozen.track_features,
        version: frozen.version,
    };
    Ok(PreparedPackage {
        directory,
        index,
        record,
    })
}

/// Write every frozen record field and the actual installed path set, without
/// scratch paths or installation timestamps. The core derives its own expected
/// receipt from the lock and archives before publication.
fn write_receipt(
    staging_dir: &Path,
    record: &LockedCondaPackage,
    files: &BTreeSet<String>,
    final_prefix: &str,
) -> HResult<MaterializedPackage> {
    let name = &record.name;
    let receipt_name = format!("{name}-{}-{}.json", record.version, record.build);
    let conda_meta = staging_dir.join("conda-meta");
    std::fs::create_dir_all(&conda_meta)
        .map_err(|error| fail("io", format!("creating {}: {error}", conda_meta.display())))?;
    let receipt_path = conda_meta.join(&receipt_name);
    let file = std::fs::File::create(&receipt_path).map_err(|error| {
        fail(
            "io",
            format!("creating {}: {error}", receipt_path.display()),
        )
    })?;
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(std::fs::Permissions::from_mode(
        gripsack_conda::receipt::FILE_MODE,
    ))
    .map_err(|error| {
        fail(
            "io",
            format!(
                "normalizing {} permissions: {error}",
                receipt_path.display()
            ),
        )
    })?;
    let mut writer = io::BufWriter::new(file);
    gripsack_conda::receipt::write(&mut writer, record, files, final_prefix)
        .map_err(|error| fail("io", format!("writing {}: {error}", receipt_path.display())))?;
    writer.flush().map_err(|error| {
        fail(
            "io",
            format!("flushing {}: {error}", receipt_path.display()),
        )
    })?;
    Ok(MaterializedPackage {
        name: name.to_string(),
        conda_meta: format!("conda-meta/{receipt_name}"),
    })
}
