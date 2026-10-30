//! Frozen PyPI policy, wheel normalization and selected dependency admission.
use super::{HResult, Selection, anonymous_url, fail, parse_spec, unsatisfied};
use gripsack_ir::workspace_model::lock::LockedWheel;
use pep508_rs::{MarkerEnvironment, MarkerEnvironmentBuilder, Requirement, VersionOrUrl};
use pixi_manifest::FeaturesExt;
use pixi_pypi_spec::PixiPypiSource;
use rattler_conda_types::{MatchSpec, RepoDataRecord};
use rattler_lock::{Environment, PypiDistributionData};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub(super) fn validate_policy(
    selected: &Selection<'_>,
    environment: &Environment<'_>,
) -> HResult<()> {
    let options = selected.pypi_options();
    if !selected.pypi_exclude_newer_config_resolved().is_empty() {
        return Err(fail(
            "unsupported_pypi_policy",
            "PyPI exclude-newer requires upload timestamps absent from the captured wheel records"
                .into(),
        ));
    }
    // These policies require build/source inputs or resolution semantics
    // which cannot be proved from captured distribution records alone.
    if options
        .find_links
        .as_ref()
        .is_some_and(|links| !links.is_empty())
        || options
            .dependency_overrides
            .as_ref()
            .is_some_and(|overrides| !overrides.is_empty())
        || options.no_binary.is_some()
        || options.skip_wheel_filename_check == Some(true)
        || options.index_strategy.is_some()
    {
        return Err(fail("unsupported_pypi_policy", "PyPI find-links, overrides, no-binary, index strategies and skipped filename checks are not supported by frozen wheel import".into()));
    }
    let mut indexes = vec![options.index_url.unwrap_or_else(|| {
        reqwest::Url::parse("https://pypi.org/simple").expect("constant PyPI index URL")
    })];
    indexes.extend(options.extra_index_urls.unwrap_or_default());
    for index in &indexes {
        anonymous_url(index)?;
    }
    let locked_indexes = environment
        .pypi_indexes()
        .ok_or_else(|| unsatisfied("PyPI lock does not retain its index selection"))?;
    if locked_indexes.indexes != indexes || !locked_indexes.find_links.is_empty() {
        return Err(unsatisfied("manifest and lock PyPI indexes differ"));
    }
    use pixi_manifest::pypi::pypi_options::PrereleaseMode as RequestedPrerelease;
    use rattler_lock::PypiPrereleaseMode as LockedPrerelease;
    let prerelease = match options.prerelease_mode.unwrap_or_default() {
        RequestedPrerelease::Disallow => LockedPrerelease::Disallow,
        RequestedPrerelease::Allow => LockedPrerelease::Allow,
        RequestedPrerelease::IfNecessary => LockedPrerelease::IfNecessary,
        RequestedPrerelease::Explicit => LockedPrerelease::Explicit,
        RequestedPrerelease::IfNecessaryOrExplicit => LockedPrerelease::IfNecessaryOrExplicit,
    };
    if environment.solve_options().pypi_prerelease_mode != prerelease {
        return Err(unsatisfied(
            "manifest and lock PyPI prerelease policies differ",
        ));
    }
    Ok(())
}

pub(super) fn normalize_wheel(package: &PypiDistributionData) -> HResult<LockedWheel> {
    let name = &package.name;
    let url = package.location.as_url().ok_or_else(|| {
        fail(
            "source_package",
            format!("local pypi package '{name}' is not an immutable HTTPS wheel"),
        )
    })?;
    let filename = url
        .path_segments()
        .and_then(|mut parts| parts.next_back())
        .filter(|name| name.ends_with(".whl"))
        .ok_or_else(|| {
            fail(
                "source_package",
                format!("pypi package '{name}' is a source archive, not a wheel"),
            )
        })?;
    anonymous_url(url)?;
    let sha256 = package
        .hash
        .as_ref()
        .and_then(|hash| hash.sha256())
        .ok_or_else(|| {
            fail(
                "missing_hash",
                format!("pypi wheel '{name}' requires SHA-256"),
            )
        })?;
    let mut requires_dist: Vec<_> = package
        .requires_dist
        .iter()
        .map(ToString::to_string)
        .collect();
    requires_dist.sort();
    requires_dist.dedup();
    Ok(LockedWheel {
        name: name.to_string(),
        version: package.version.to_string(),
        filename: filename.into(),
        url: url.to_string(),
        sha256: super::super::hex_lower(sha256),
        // Rattler's selected PyPI lock model does not retain an archive size.
        size: None,
        requires_python: package.requires_python.as_ref().map(ToString::to_string),
        requires_dist,
    })
}

/// Validate the selected PyPI graph without an index lookup or a name-mapping
/// service. Only explicit Conda purls may satisfy Python requirements.
pub(super) fn validate_pypi_closure(
    selected: &Selection<'_>,
    platform: &pixi_manifest::PixiPlatform,
    records: &[RepoDataRecord],
    packages: &[&PypiDistributionData],
) -> HResult<Vec<MatchSpec>> {
    let dependencies = selected.pypi_dependencies(Some(platform));
    if packages.is_empty() && dependencies.names().next().is_none() {
        return Ok(Vec::new());
    }
    let python = records
        .iter()
        .find(|record| record.package_record.name.as_normalized() == "python")
        .ok_or_else(|| unsatisfied("pypi wheels require a locked Conda python interpreter"))?;
    let markers = pypi_markers(platform, python)?;
    let mut pending = VecDeque::new();
    let mut indexes = BTreeMap::new();
    for (name, spec) in dependencies.into_specs() {
        let version_or_url = match &spec.source {
            PixiPypiSource::Registry { version, index } => {
                if let Some(index) = index {
                    anonymous_url(index)?;
                    if indexes
                        .insert(name.as_normalized().clone(), index.clone())
                        .is_some_and(|previous| previous != *index)
                    {
                        return Err(unsatisfied(format!(
                            "conflicting PyPI indexes for '{}'",
                            name.as_source()
                        )));
                    }
                }
                version.clone().into()
            }
            PixiPypiSource::Url { url, subdirectory }
                if subdirectory.is_empty() && url.path().ends_with(".whl") =>
            {
                anonymous_url(url)?;
                Some(VersionOrUrl::Url(pep508_rs::VerbatimUrl::from_url(
                    url.clone(),
                )))
            }
            _ => {
                return Err(fail(
                    "source_package",
                    format!(
                        "manifest source/editable pypi package '{}' requires a build capability",
                        name.as_source()
                    ),
                ));
            }
        };
        let requirement = Requirement {
            name: name.as_normalized().clone(),
            extras: spec.extras.clone(),
            version_or_url,
            marker: spec.env_markers.clone(),
            origin: None,
        };
        if requirement.marker.evaluate(&markers, &[]) {
            pending.push_back(requirement);
        }
    }
    let mut names = BTreeMap::new();
    for (index, package) in packages.iter().enumerate() {
        if names.insert(&package.name, index).is_some() {
            return Err(fail(
                "invalid_closure",
                format!("duplicate pypi wheel '{}'", package.name),
            ));
        }
        if let Some(expected) = indexes.get(&package.name)
            && package.index_url.as_ref() != Some(expected)
        {
            return Err(unsatisfied(format!(
                "pypi package '{}' has a stale index selection",
                package.name
            )));
        }
    }
    let mut conda_names = BTreeMap::new();
    for (index, record) in records.iter().enumerate() {
        for purl in record.package_record.purls.iter().flatten() {
            if purl.package_type() != "pypi" {
                continue;
            }
            let name = purl
                .name()
                .parse::<pep508_rs::PackageName>()
                .map_err(|error| {
                    fail(
                        "invalid_record",
                        format!("invalid PyPI purl '{purl}': {error}"),
                    )
                })?;
            if names.contains_key(&name)
                || conda_names
                    .insert(name.clone(), (index, purl.version()))
                    .is_some()
            {
                return Err(fail(
                    "invalid_closure",
                    format!("multiple Conda/PyPI owners of '{name}'"),
                ));
            }
        }
    }
    let mut reached = BTreeSet::new();
    let mut expanded = BTreeSet::new();
    let mut conda_roots = BTreeSet::new();
    while let Some(requirement) = pending.pop_front() {
        let name = &requirement.name;
        if let Some(&index) = names.get(name) {
            let package = packages[index];
            let satisfied = match &requirement.version_or_url {
                None => true,
                Some(VersionOrUrl::VersionSpecifier(spec)) => spec.contains(&package.version),
                Some(VersionOrUrl::Url(url)) => package.location.as_url() == Some(&url.to_url()),
            };
            if !satisfied {
                return Err(unsatisfied(format!(
                    "pypi wheel '{name}' does not satisfy '{requirement}'"
                )));
            }
            reached.insert(index);
            // Expand separately for each requested extra set. This also handles
            // a new extra arriving later through a dependency cycle.
            if !expanded.insert((index, requirement.extras.clone())) {
                continue;
            }
            for dependency in &package.requires_dist {
                if dependency.marker.evaluate(&markers, &requirement.extras) {
                    pending.push_back(dependency.clone());
                }
            }
        } else if let Some(&(index, purl_version)) = conda_names.get(name) {
            let record = &records[index].package_record;
            let version = if let Some(version) = purl_version {
                version.parse()
            } else {
                record.version.to_string().parse()
            }
            .map_err(|error| {
                unsatisfied(format!(
                    "Conda Python provider '{name}' has invalid PEP 440 version: {error}"
                ))
            })?;
            let satisfied = match &requirement.version_or_url {
                None => true,
                Some(VersionOrUrl::VersionSpecifier(spec)) => spec.contains(&version),
                Some(VersionOrUrl::Url(_)) => false,
            };
            if !satisfied {
                return Err(unsatisfied(format!(
                    "Conda Python provider '{name}' does not satisfy '{requirement}'"
                )));
            }
            conda_roots.insert(record.name.as_normalized());
        } else {
            return Err(unsatisfied(format!(
                "missing locked pypi dependency '{requirement}'"
            )));
        }
    }
    for (index, package) in packages.iter().enumerate() {
        if !reached.contains(&index) {
            return Err(fail(
                "invalid_closure",
                format!("unreachable pypi wheel '{}'", package.name),
            ));
        }
    }
    conda_roots.into_iter().map(parse_spec).collect()
}

fn pypi_markers(
    platform: &pixi_manifest::PixiPlatform,
    python: &RepoDataRecord,
) -> HResult<MarkerEnvironment> {
    // Match Pixi's target-derived marker environment; never consult host Python.
    let (sys_platform, platform_system, os_name, platform_machine) =
        match platform.subdir().as_str() {
            "linux-64" => ("linux", "Linux", "posix", "x86_64"),
            "linux-aarch64" => ("linux", "Linux", "posix", "aarch64"),
            "osx-64" => ("darwin", "Darwin", "posix", "x86_64"),
            "osx-arm64" => ("darwin", "Darwin", "posix", "arm64"),
            other => {
                return Err(fail(
                    "unsupported_platform",
                    format!("PyPI marker target '{other}' is unsupported"),
                ));
            }
        };
    let full = python.package_record.version.to_string();
    let (major, minor) = python
        .package_record
        .version
        .version()
        .as_major_minor()
        .ok_or_else(|| unsatisfied(format!("invalid locked Python version '{full}'")))?;
    MarkerEnvironment::try_from(MarkerEnvironmentBuilder {
        implementation_name: "cpython",
        implementation_version: &full,
        os_name,
        platform_machine,
        platform_python_implementation: "CPython",
        platform_release: "",
        platform_system,
        platform_version: "",
        python_full_version: &full,
        python_version: &format!("{major}.{minor}"),
        sys_platform,
    })
    .map_err(|error| unsatisfied(format!("invalid locked Python marker environment: {error}")))
}
