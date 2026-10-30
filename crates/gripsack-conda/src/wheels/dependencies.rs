//! Distribution ownership and PEP 508 closure admission, independent of the
//! installer. Conda project names are never guessed from Conda package names.

use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::str::FromStr;

use gripsack_ir::workspace_model::lock::LockedCondaEnvironment;
use pep508_rs::pep440_rs::{Version, VersionSpecifiers};
use pep508_rs::{
    ExtraName, MarkerEnvironment, MarkerEnvironmentBuilder, MarkerTree, MarkerTreeKind,
    MarkerValueString, MarkerWarningKind, PackageName, Pep508Url, Requirement, VerbatimUrl,
    VersionOrUrl,
};

use super::{PythonTarget, WheelError, invalid};

/// Parsed installed project metadata, not an inferred Conda-to-PyPI mapping.
pub struct DistributionMetadata {
    name: String,
    version: Version,
    requires_python: Option<VersionSpecifiers>,
    requirements: Vec<Requirement>,
    wheel_url: Option<String>,
}

impl DistributionMetadata {
    /// Parsing is bounded by the same metadata budget as helper protocol input.
    pub fn parse(bytes: &[u8]) -> Result<Self, WheelError> {
        if bytes.len() as u64 > crate::protocol::MAX_REQUEST_BYTES {
            return Err(invalid(
                "Python distribution metadata exceeds admission budget",
            ));
        }
        let metadata = uv_pypi_types::Metadata23::parse(bytes).map_err(invalid)?;
        Self::from_fields(
            &metadata.name,
            &metadata.version,
            metadata.requires_python.as_deref(),
            &metadata.requires_dist,
            None,
        )
    }

    fn from_fields(
        name: &str,
        version: &str,
        requires_python: Option<&str>,
        requirements: &[String],
        wheel_url: Option<String>,
    ) -> Result<Self, WheelError> {
        let name = PackageName::from_str(name).map_err(invalid)?.to_string();
        let version = Version::from_str(version).map_err(invalid)?;
        let requires_python = requires_python
            .map(VersionSpecifiers::from_str)
            .transpose()
            .map_err(invalid)?;
        let requirements = requirements
            .iter()
            .map(|value| {
                // Parsing can discard invalid comparisons while returning a
                // requirement. Refuse those diagnostics before simplification
                // can hide an ambiguous or unsupported condition.
                let mut warnings = Vec::new();
                let requirement: Requirement =
                    Requirement::parse_reporter(value, "/", &mut |_, warning| {
                        warnings.push(warning)
                    })
                    .map_err(invalid)?;
                if !warnings.is_empty() {
                    return Err(invalid(format!(
                        "ambiguous requirement '{value}': {}",
                        warnings.join("; ")
                    )));
                }
                // The reporting API requires a working directory. Preserve
                // FromStr's no-base URL policy even if another crate enables
                // relative-path extensions through feature unification.
                if let Some(VersionOrUrl::Url(url)) = &requirement.version_or_url {
                    <VerbatimUrl as Pep508Url>::parse_url(
                        url.given().expect("parsed requirement retains its URL"),
                        None,
                    )
                    .map_err(invalid)?;
                }
                reject_unfrozen_markers(&requirement.marker)?;
                Ok(requirement)
            })
            .collect::<Result<_, WheelError>>()?;
        Ok(Self {
            name,
            version,
            requires_python,
            requirements,
            wheel_url,
        })
    }
}

/// Check a complete environment against actual Conda-owned metadata bytes.
/// The caller must obtain these bytes from admitted archive paths, never from
/// the mutable installed prefix. Wheel metadata was independently bound to its
/// original archive by `WheelArchive::admit` before this check.
pub fn validate_dependencies(
    closure: &LockedCondaEnvironment,
    conda_metadata: &[(String, Vec<u8>)],
) -> Result<(), WheelError> {
    if closure.wheels.is_empty() {
        return Ok(());
    }
    let target = PythonTarget::from_lock(closure)?;
    let mut distributions = BTreeMap::new();
    for (path, bytes) in conda_metadata {
        let distribution = DistributionMetadata::parse(bytes)
            .map_err(|error| invalid(format!("Conda-owned metadata '{path}': {error}")))?;
        insert_distribution(&mut distributions, distribution)?;
    }
    for wheel in &closure.wheels {
        insert_distribution(
            &mut distributions,
            DistributionMetadata::from_fields(
                &wheel.name,
                &wheel.version,
                wheel.requires_python.as_deref(),
                &wheel.requires_dist,
                Some(wheel.url.clone()),
            )?,
        )?;
    }
    validate_distribution_graph(&target, &distributions)
}

fn insert_distribution(
    distributions: &mut BTreeMap<String, DistributionMetadata>,
    distribution: DistributionMetadata,
) -> Result<(), WheelError> {
    if distributions.contains_key(&distribution.name) {
        return Err(invalid(format!(
            "Python distribution '{}' has multiple Conda/wheel owners",
            distribution.name
        )));
    }
    distributions.insert(distribution.name.clone(), distribution);
    Ok(())
}

fn validate_distribution_graph(
    target: &PythonTarget,
    distributions: &BTreeMap<String, DistributionMetadata>,
) -> Result<(), WheelError> {
    let python_full_version = target.version.to_string();
    let python_version = format!("{}.{}", target.short_version.0, target.short_version.1);
    let environment = MarkerEnvironment::try_from(MarkerEnvironmentBuilder {
        implementation_name: "cpython",
        implementation_version: &python_full_version,
        os_name: "posix",
        platform_machine: "x86_64",
        platform_python_implementation: "CPython",
        // Admission rejects markers depending on these unfrozen host facts.
        platform_release: "",
        platform_version: "",
        platform_system: "Linux",
        python_full_version: &python_full_version,
        python_version: &python_version,
        sys_platform: "linux",
    })
    .map_err(invalid)?;
    let mut extras: BTreeMap<&str, BTreeSet<ExtraName>> = BTreeMap::new();
    let mut pending: VecDeque<&str> = distributions.keys().map(String::as_str).collect();
    while let Some(name) = pending.pop_front() {
        let distribution = &distributions[name];
        if distribution
            .requires_python
            .as_ref()
            .is_some_and(|spec| !spec.contains(&target.version))
        {
            return Err(invalid(format!(
                "distribution '{name}' rejects locked Python {}",
                target.version
            )));
        }
        let selected: Vec<_> = extras.get(name).into_iter().flatten().cloned().collect();
        for requirement in &distribution.requirements {
            let (base_active, base_warnings) =
                requirement.evaluate_markers_and_report(&environment, &[]);
            let (extra_active, extra_warnings) =
                requirement.evaluate_markers_and_report(&environment, &selected);
            // pep508_rs 0.9.2 reports lexicographic comparisons for the
            // complement ranges of even string equality nodes, before checking
            // which edge matches. These are deterministic string comparisons,
            // not admission errors. Invalid source comparisons were rejected
            // by the parsing reporter above.
            if base_warnings
                .iter()
                .chain(&extra_warnings)
                .any(|(kind, _)| *kind != MarkerWarningKind::LexicographicComparison)
            {
                return Err(invalid(format!(
                    "distribution '{name}' has ambiguous requirement '{requirement}'"
                )));
            }
            if !base_active && !extra_active {
                continue;
            }
            let dependency = distributions
                .get(requirement.name.as_ref())
                .ok_or_else(|| {
                    invalid(format!(
                        "distribution '{name}' needs '{requirement}', absent from frozen closure"
                    ))
                })?;
            match &requirement.version_or_url {
                Some(VersionOrUrl::VersionSpecifier(specifier))
                    if !specifier.contains(&dependency.version) =>
                {
                    return Err(invalid(format!(
                        "distribution '{name}' needs '{requirement}', selected {}",
                        dependency.version
                    )));
                }
                Some(VersionOrUrl::Url(url))
                    if dependency.wheel_url.as_deref() != Some(url.to_string().as_str()) =>
                {
                    return Err(invalid(format!(
                        "distribution '{name}' references an unretained URL in '{requirement}'"
                    )));
                }
                _ => {}
            }
            let activated = extras.entry(dependency.name.as_str()).or_default();
            let mut changed = false;
            for extra in &requirement.extras {
                changed |= activated.insert(extra.clone());
            }
            if changed {
                pending.push_back(dependency.name.as_str());
            }
        }
    }
    Ok(())
}

// Traverse the maintained parser's decision graph rather than expanding DNF;
// shared subtrees are visited once even for adversarial marker expressions.
fn reject_unfrozen_markers(marker: &MarkerTree) -> Result<(), WheelError> {
    let mut pending = vec![marker.clone()];
    let mut visited = HashSet::new();
    while let Some(tree) = pending.pop() {
        if !visited.insert(tree.clone()) {
            continue;
        }
        match tree.kind() {
            MarkerTreeKind::True | MarkerTreeKind::False => {}
            MarkerTreeKind::Version(node) => pending.extend(node.edges().map(|(_, child)| child)),
            MarkerTreeKind::Extra(node) => pending.extend(node.children().map(|(_, child)| child)),
            MarkerTreeKind::String(node) => {
                admit_marker_key(node.key())?;
                pending.extend(node.children().map(|(_, child)| child));
            }
            MarkerTreeKind::In(node) => {
                admit_marker_key(node.key())?;
                pending.extend(node.children().map(|(_, child)| child));
            }
            MarkerTreeKind::Contains(node) => {
                admit_marker_key(node.key())?;
                pending.extend(node.children().map(|(_, child)| child));
            }
        }
    }
    Ok(())
}

fn admit_marker_key(key: &MarkerValueString) -> Result<(), WheelError> {
    if matches!(
        key,
        MarkerValueString::PlatformRelease
            | MarkerValueString::PlatformVersion
            | MarkerValueString::PlatformVersionDeprecated
    ) {
        return Err(invalid(
            "dependency markers requiring an unfrozen kernel release/version are unsupported",
        ));
    }
    Ok(())
}
