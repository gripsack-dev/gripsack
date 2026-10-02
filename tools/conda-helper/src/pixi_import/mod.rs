//! Offline admission of a captured Pixi manifest/lock pair. Pixi owns manifest
//! selection and spec conversion; Rattler owns record and MatchSpec semantics.
use super::{HResult, fail, normalize_record};
use gripsack_conda::protocol::ImportPixiRequest;
use gripsack_ir::workspace_v6::lock::{
    BytecodePolicy, ChannelPriority, LockedCondaEnvironment, LockedCondaSystemRequirements,
    LockedVirtualPackage, LockedVirtualPackageRequirement, MaterializerPolicy, ReceiptPolicy,
};
use pixi_manifest::{
    Feature, FeatureName, FeaturesExt, HasFeaturesIter, HasWorkspaceManifest, SystemRequirements,
    WorkspaceManifest,
};
use rattler_conda_types::{
    ChannelConfig, GenericVirtualPackage, MatchSpec, MatchSpecCondition, Matches,
    ParseMatchSpecOptions, ParseStrictness, RepoDataRecord,
};
use rattler_lock::LockFile;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::Path,
};

struct Selection<'a> {
    manifest: &'a WorkspaceManifest,
    features: &'a [&'a Feature],
}
impl<'a> HasWorkspaceManifest<'a> for Selection<'a> {
    fn workspace_manifest(&self) -> &'a WorkspaceManifest {
        self.manifest
    }
}
impl<'a> HasFeaturesIter<'a> for Selection<'a> {
    fn features(&self) -> impl DoubleEndedIterator<Item = &'a Feature> + 'a {
        self.features.iter().copied()
    }
}

fn invalid_manifest(error: impl std::fmt::Display) -> super::Failure {
    fail("invalid_manifest", error.to_string())
}
fn unsatisfied(message: impl Into<String>) -> super::Failure {
    fail("unsatisfied_manifest", message.into())
}
fn channel(raw: &str) -> HResult<String> {
    if let Ok(url) = reqwest::Url::parse(raw) {
        anonymous_url(&url)?;
    }
    let canonical = gripsack_conda::channels::canonicalize_channel(raw)
        .map_err(|error| fail("invalid_channel", error.to_string()))?;
    let url = reqwest::Url::parse(&canonical)
        .map_err(|error| fail("invalid_channel", error.to_string()))?;
    anonymous_url(&url)?;
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

fn anonymous_url(url: &reqwest::Url) -> HResult<()> {
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url
            .path_segments()
            .is_some_and(|mut segments| segments.any(|part| part == "t"))
    {
        return Err(fail(
            "credentials_rejected",
            "authenticated/tokenized channels and artifact URLs are not importable".into(),
        ));
    }
    if url.scheme() != "https" {
        return Err(fail(
            "invalid_channel",
            "anonymous HTTPS is required".into(),
        ));
    }
    Ok(())
}

pub(super) fn import_pixi(request: ImportPixiRequest) -> HResult<LockedCondaEnvironment> {
    // The root is only an anchor for upstream syntax conversion. No input is
    // discovered or read from it; local/source artifacts are refused below.
    let (manifest, raw_requirements) = parse_manifest(&request.manifest_bytes)?;
    let declared = manifest
        .environment(request.environment.as_str())
        .ok_or_else(|| {
            fail(
                "unknown_environment",
                format!("manifest has no environment '{}'", request.environment),
            )
        })?;
    let mut features = declared
        .features
        .iter()
        .map(|name| {
            manifest
                .feature(name)
                .ok_or_else(|| invalid_manifest(format!("unknown feature '{name}'")))
        })
        .collect::<HResult<Vec<_>>>()?;
    if !declared.no_default_feature {
        features.push(manifest.default_feature());
    }
    let selected = Selection {
        manifest: &manifest,
        features: &features,
    };
    let supported = selected.platforms();
    let candidates: Vec<_> = manifest
        .workspace
        .platforms
        .iter()
        .filter(|platform| {
            supported.contains(platform.name()) && platform.subdir().as_str() == request.platform
        })
        .collect();
    let platform = match candidates.as_slice() {
        [platform] => *platform,
        [] => {
            return Err(fail(
                "unknown_platform",
                format!(
                    "manifest environment '{}' does not support '{}'",
                    request.environment, request.platform
                ),
            ));
        }
        _ => {
            return Err(fail(
                "ambiguous_platform",
                format!(
                    "manifest has multiple platform variants for '{}'",
                    request.platform
                ),
            ));
        }
    };
    let requirements = selected
        .features()
        .filter_map(|feature| raw_requirements.get(&feature.name))
        .try_fold(SystemRequirements::default(), |requirements, next| {
            requirements.union(next)
        })
        .map_err(invalid_manifest)?;
    let system_requirements = system_requirements(platform, &requirements)?;
    let lock = LockFile::from_reader(request.lock_bytes.as_bytes(), None)
        .map_err(|error| fail("invalid_lock", format!("pixi lock does not parse: {error}")))?;
    let environment = lock.environment(&request.environment).ok_or_else(|| {
        fail(
            "unknown_environment",
            format!("lock has no environment '{}'", request.environment),
        )
    })?;
    let locked_platform = lock
        .platform(platform.name().as_str())
        .or_else(|| lock.platform(&request.platform))
        .ok_or_else(|| {
            fail(
                "unknown_platform",
                format!("lock has no platform '{}'", platform.name()),
            )
        })?;
    if locked_platform.subdir() != platform.subdir() {
        return Err(unsatisfied("manifest and lock platform subdirs differ"));
    }
    let packages = environment.packages(locked_platform).ok_or_else(|| {
        fail(
            "unknown_platform",
            format!(
                "lock environment '{}' has no platform '{}'",
                request.environment,
                platform.name()
            ),
        )
    })?;
    // Refuse records before conversion, so diagnostics retain their exact names.
    for package in packages {
        if package.as_pypi().is_some() {
            return Err(fail(
                "pypi_package",
                format!(
                    "pypi package '{}' is not importable into a conda closure",
                    package.name()
                ),
            ));
        }
        if package
            .as_conda()
            .is_some_and(|conda| conda.as_binary().is_none())
        {
            return Err(fail(
                "source_package",
                format!(
                    "source (path/git/url) package '{}' is not importable into a conda closure",
                    package.name()
                ),
            ));
        }
    }
    if let Some(name) = selected.pypi_dependencies(Some(platform)).names().next() {
        return Err(fail(
            "pypi_package",
            format!(
                "manifest pypi package '{}' is not importable into a conda closure",
                name.as_source()
            ),
        ));
    }
    if let Some(name) = selected
        .combined_dev_dependencies(Some(platform))
        .names()
        .next()
    {
        return Err(fail(
            "source_package",
            format!(
                "manifest source package '{}' is not importable into a conda closure",
                name.as_source()
            ),
        ));
    }
    let channels = selected
        .channels()
        .into_iter()
        .map(|raw| channel(&raw.to_string()))
        .collect::<HResult<Vec<_>>>()?;
    let locked_channels = environment
        .channels()
        .iter()
        .map(|raw| {
            if !raw.used_env_vars.is_empty() {
                return Err(fail(
                    "credentials_rejected",
                    "lock channel requires environment-variable authentication".into(),
                ));
            }
            channel(&raw.url)
        })
        .collect::<HResult<Vec<_>>>()?;
    if channels != locked_channels {
        return Err(unsatisfied(
            "manifest and lock channel priority lists differ",
        ));
    }
    let priority = selected
        .channel_priority()
        .map_err(invalid_manifest)?
        .unwrap_or_default();
    if environment.solve_options().channel_priority != priority.into()
        || environment.solve_options().strategy != selected.solve_strategy().into()
    {
        return Err(unsatisfied("manifest and lock solver policies differ"));
    }
    let channel_priority = match priority {
        pixi_manifest::ChannelPriority::Strict => ChannelPriority::Strict,
        pixi_manifest::ChannelPriority::Flexible => ChannelPriority::Flexible,
        pixi_manifest::ChannelPriority::Disabled => {
            return Err(fail(
                "unsupported_channel_priority",
                "disabled channel priority is not a portable frozen policy".into(),
            ));
        }
    };
    let config = ChannelConfig::default_with_root_dir(Path::new("/").to_path_buf());
    let records = environment
        .conda_repodata_records(locked_platform)
        .map_err(|error| fail("invalid_record", error.to_string()))?
        .unwrap_or_default();
    let mut names = BTreeMap::new();
    let mut artifacts = BTreeSet::new();
    for (index, record) in records.iter().enumerate() {
        anonymous_url(&record.url)?;
        let name = record.package_record.name.as_normalized();
        let identifier = &record.identifier.identifier;
        if identifier.name != name
            || identifier.version != record.package_record.version.to_string()
            || identifier.build_string != record.package_record.build
        {
            return Err(fail(
                "invalid_closure",
                format!("package '{name}' record contradicts its immutable archive filename"),
            ));
        }
        if name.starts_with("__") || names.insert(name, index).is_some() {
            return Err(fail(
                "invalid_closure",
                format!("duplicate or virtual package record '{name}'"),
            ));
        }
        if record.package_record.subdir != request.platform
            && record.package_record.subdir != "noarch"
        {
            return Err(fail(
                "invalid_closure",
                format!(
                    "package '{name}' targets '{}' instead of '{}'",
                    record.package_record.subdir, request.platform
                ),
            ));
        }
        if !artifacts.insert(record.url.as_str()) {
            return Err(fail(
                "invalid_closure",
                format!("package '{name}' repeats an artifact URL"),
            ));
        }
    }
    let mut virtuals = locked_platform
        .virtual_packages()
        .iter()
        .map(|raw| {
            pixi_manifest::platform::parse_locked_virtual_package(raw).ok_or_else(|| {
                fail(
                    "invalid_lock",
                    format!("invalid recorded virtual package '{raw}'"),
                )
            })
        })
        .collect::<HResult<Vec<_>>>()?;
    virtuals.sort_by(|left, right| left.name.cmp(&right.name));
    let mut virtual_names = BTreeSet::new();
    for fact in &virtuals {
        if !fact.name.as_normalized().starts_with("__") || !virtual_names.insert(&fact.name) {
            return Err(fail(
                "invalid_lock",
                format!(
                    "invalid or duplicate virtual package '{}'",
                    fact.name.as_normalized()
                ),
            ));
        }
    }
    let declared_virtuals: BTreeSet<_> = platform
        .customised_virtual_packages()
        .into_iter()
        .map(|fact| fact.to_string())
        .collect();
    let recorded_virtuals: BTreeSet<_> = virtuals
        .iter()
        .filter(|fact| !pixi_manifest::platform::is_subdir_default(fact, platform.subdir()))
        .map(ToString::to_string)
        .collect();
    if !virtuals.is_empty() && declared_virtuals != recorded_virtuals {
        return Err(unsatisfied(
            "manifest and lock virtual-package declarations differ",
        ));
    }
    let mut roots = Vec::new();
    let mut constraints = Vec::new();
    for (dependencies, destination) in [
        (selected.combined_dependencies(Some(platform)), &mut roots),
        (
            selected.combined_constraints(Some(platform)),
            &mut constraints,
        ),
    ] {
        for (name, spec) in dependencies.into_specs() {
            if spec.is_source() || spec.as_path_binary().is_some() {
                return Err(fail(
                    "source_package",
                    format!(
                        "manifest source/path package '{}' is not importable into a conda closure",
                        name.as_normalized()
                    ),
                ));
            }
            let spec = spec
                .to_match_spec(&name, &config)
                .map_err(invalid_manifest)?;
            if let Some(url) = &spec.url {
                super::reject_credentials(url, "manifest artifact URL")?;
            }
            if let Some(raw) = &spec.channel {
                channel(raw.base_url.as_str())?;
            }
            destination.push(spec);
        }
    }
    validate_closure(&records, &names, &virtuals, roots, constraints)?;
    let mut normalized = records
        .iter()
        .map(|record| normalize_record(record, &channels))
        .collect::<HResult<Vec<_>>>()?;
    for package in &normalized {
        if !channels.contains(&package.channel)
            || !package
                .url
                .starts_with(&format!("{}/{}/", package.channel, package.subdir))
        {
            return Err(fail(
                "invalid_closure",
                format!(
                    "package '{}' artifact is outside the selected channel/subdir",
                    package.name
                ),
            ));
        }
    }
    // Exclude-newer uses the canonical upstream timestamp/override semantics.
    if let Some(cutoffs) = selected
        .exclude_newer_config_resolved(&config)
        .map_err(invalid_manifest)?
    {
        let cutoffs: rattler_solve::ExcludeNewer = cutoffs.into();
        for record in &records {
            if cutoffs.is_excluded(record) {
                return Err(unsatisfied(format!(
                    "package '{}' violates exclude-newer",
                    record.package_record.name.as_normalized()
                )));
            }
        }
    }
    normalized.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(LockedCondaEnvironment {
        platform: request.platform,
        channels,
        channel_priority,
        system_requirements,
        // Only captured lock facts, never host detection or Pixi defaults.
        virtual_packages: virtuals
            .into_iter()
            .map(|fact| LockedVirtualPackage {
                name: fact.name.as_normalized().into(),
                version: fact.version.to_string(),
                build: fact.build_string,
            })
            .collect(),
        packages: normalized,
        materializer: MaterializerPolicy {
            bytecode: BytecodePolicy::Suppress,
            receipt: ReceiptPolicy::NormalizedCondaMeta,
        },
    })
}

type RawRequirements = BTreeMap<FeatureName, SystemRequirements>;

fn raw_requirements(raw: &pixi_manifest::toml::TomlManifest) -> RawRequirements {
    let mut requirements = BTreeMap::new();
    if let Some(default) = &raw.system_requirements {
        requirements.insert(FeatureName::Default, default.value.clone());
    }
    if let Some(features) = &raw.feature {
        requirements.extend(
            features
                .value
                .iter()
                .map(|(name, feature)| (name.value.clone(), feature.system_requirements.clone())),
        );
    }
    requirements
}

fn parse_manifest(bytes: &str) -> HResult<(WorkspaceManifest, RawRequirements)> {
    use pixi_manifest::{
        pyproject::PyProjectManifest,
        toml::{ExternalWorkspaceProperties, FromTomlStr, PackageDefaults, TomlManifest},
    };
    // Both entry points parse captured strings; neither discovers a workspace.
    if let Ok(pyproject) = PyProjectManifest::from_toml_str(bytes) {
        if let Some(raw) = pyproject.tool.as_ref().and_then(|tool| tool.pixi.as_ref()) {
            let requirements = raw_requirements(raw);
            return pyproject
                .into_workspace_manifest(Path::new("/"))
                .map(|parsed| (parsed.0, requirements))
                .map_err(invalid_manifest);
        }
    }
    let raw = TomlManifest::from_toml_str(bytes).map_err(invalid_manifest)?;
    let requirements = raw_requirements(&raw);
    raw.into_workspace_manifest(
        ExternalWorkspaceProperties::default(),
        PackageDefaults::default(),
        Path::new("/"),
    )
    .map(|parsed| (parsed.0, requirements))
    .map_err(invalid_manifest)
}

fn system_requirements(
    platform: &pixi_manifest::PixiPlatform,
    legacy: &SystemRequirements,
) -> HResult<LockedCondaSystemRequirements> {
    let mut requirements = BTreeMap::new();
    let mut archspec = legacy.archspec.clone();
    for requirement in platform
        .customised_virtual_packages()
        .into_iter()
        .chain(legacy.to_declared_virtual_packages())
    {
        let name = requirement.name.as_normalized();
        let applies = match name {
            "__linux" | "__glibc" | "__musl" | "__eglibc" => platform.subdir().is_linux(),
            "__osx" => platform.subdir().is_osx(),
            "__cuda" => !platform.subdir().is_osx(),
            _ => true,
        };
        if !applies {
            continue;
        }
        if name == "__archspec" {
            if archspec
                .as_ref()
                .is_some_and(|arch| arch != &requirement.build_string)
            {
                return Err(invalid_manifest("conflicting architecture requirements"));
            }
            archspec = Some(requirement.build_string);
        } else {
            requirements.insert(
                name.to_string(),
                LockedVirtualPackageRequirement {
                    name: name.to_string(),
                    minimum_version: requirement.version.to_string(),
                    build: (!requirement.build_string.is_empty())
                        .then_some(requirement.build_string),
                },
            );
        }
    }
    if let Some(archspec) = &archspec {
        pixi_manifest::platform::validate_archspec_name(archspec).map_err(invalid_manifest)?;
    }
    Ok(LockedCondaSystemRequirements {
        virtual_packages: requirements.into_values().collect(),
        archspec,
    })
}

fn parse_spec(text: &str) -> HResult<MatchSpec> {
    MatchSpec::from_str(
        text,
        ParseMatchSpecOptions::from(ParseStrictness::Lenient)
            .with_conditionals(true)
            .with_extras(true)
            .with_flags(true),
    )
    .map_err(|error| {
        fail(
            "invalid_record",
            format!("invalid dependency '{text}': {error}"),
        )
    })
}

fn condition_holds(
    condition: &MatchSpecCondition,
    records: &[RepoDataRecord],
    virtuals: &[GenericVirtualPackage],
) -> HResult<bool> {
    Ok(match condition {
        MatchSpecCondition::MatchSpec(spec) => {
            // Captured virtual declarations are not a complete host inventory.
            // An omitted fact cannot decide a conditional either way.
            if spec.name.as_exact().is_some_and(|name| {
                name.as_normalized().starts_with("__")
                    && !virtuals.iter().any(|fact| &fact.name == name)
            }) {
                return Err(fail(
                    "missing_virtual_facts",
                    format!("conditional '{spec}' needs lock-recorded virtual facts"),
                ));
            }
            let matched = records
                .iter()
                .any(|record| gripsack_conda::records::matches_repository(spec, record))
                || virtuals.iter().any(|fact| spec.matches(fact));
            matched
                && match &spec.condition {
                    Some(condition) => condition_holds(condition, records, virtuals)?,
                    None => true,
                }
        }
        MatchSpecCondition::And(left, right) => {
            condition_holds(left, records, virtuals)? && condition_holds(right, records, virtuals)?
        }
        MatchSpecCondition::Or(left, right) => {
            condition_holds(left, records, virtuals)? || condition_holds(right, records, virtuals)?
        }
    })
}

fn validate_closure(
    records: &[RepoDataRecord],
    names: &BTreeMap<&str, usize>,
    virtuals: &[GenericVirtualPackage],
    roots: Vec<MatchSpec>,
    constraints: Vec<MatchSpec>,
) -> HResult<()> {
    let mut pending: VecDeque<_> = roots
        .into_iter()
        .map(|spec| (spec, false))
        .chain(constraints.into_iter().map(|spec| (spec, true)))
        .collect();
    let mut reached = BTreeSet::new();
    let mut extras = BTreeSet::new();
    while let Some((spec, constraint)) = pending.pop_front() {
        if let Some(condition) = &spec.condition {
            if !condition_holds(condition, records, virtuals)? {
                continue;
            }
        }
        let name = spec.name.as_exact().ok_or_else(|| {
            fail(
                "invalid_record",
                format!("dependency '{spec}' must have an exact name"),
            )
        })?;
        let name = name.as_normalized();
        if name.starts_with("__") {
            // Pixi can omit default virtuals even in modern locks. Retain
            // their requirements for core runtime admission, not guessed facts.
            let Some(fact) = virtuals
                .iter()
                .find(|fact| fact.name.as_normalized() == name)
            else {
                continue;
            };
            if !spec.matches(fact) {
                return Err(unsatisfied(format!(
                    "recorded virtual packages do not satisfy '{spec}'"
                )));
            }
            continue;
        }
        let Some(&index) = names.get(name) else {
            if constraint {
                continue;
            }
            return Err(unsatisfied(format!("locked closure is missing '{spec}'")));
        };
        let record = &records[index];
        if !gripsack_conda::records::matches_repository(&spec, record) {
            return Err(unsatisfied(format!(
                "locked package '{name}' does not satisfy '{spec}'"
            )));
        }
        if constraint {
            continue;
        }
        if reached.insert(index) {
            for text in &record.package_record.depends {
                pending.push_back((parse_spec(text)?, false));
            }
            for text in &record.package_record.constrains {
                pending.push_back((parse_spec(text)?, true));
            }
        }
        for extra in spec.extras.iter().flatten() {
            if !extras.insert((index, extra.clone())) {
                continue;
            }
            let dependencies = record
                .package_record
                .extra_depends
                .get(extra)
                .ok_or_else(|| unsatisfied(format!("package '{name}' has no extra '{extra}'")))?;
            for text in dependencies {
                pending.push_back((parse_spec(text)?, false));
            }
        }
    }
    if reached.len() != records.len() {
        let unused = records
            .iter()
            .enumerate()
            .filter(|(index, _)| !reached.contains(index))
            .map(|(_, record)| record.package_record.name.as_normalized())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(fail(
            "invalid_closure",
            format!("lock contains packages outside the selected manifest closure: {unused}"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
