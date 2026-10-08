//! Canonical ordinary and virtual requirements for a coherent frozen closure.
//! Recorded solve facts, physical native facts and partial image capabilities
//! have deliberately different absence semantics.
use crate::records::{self, RecordError};
use gripsack_ir::workspace_model::lock::{
    ChannelPriority, LockedCondaEnvironment, LockedCondaPackage, LockedCondaSystemRequirements,
    LockedVirtualPackage,
};
use rattler_conda_types::{
    GenericVirtualPackage, MatchSpec, MatchSpecCondition, Matches, PackageName,
    ParseMatchSpecOptions, ParseStrictness, RepoDataRecord, Version,
};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    str::FromStr,
};

#[derive(Debug, thiserror::Error)]
pub enum VirtualConstraintError {
    #[error(transparent)]
    Record(#[from] RecordError),
    #[error("invalid measured virtual package {name:?}: {detail}")]
    Fact { name: String, detail: String },
    #[error("package {package:?}: invalid constraint {spec:?}: {detail}")]
    Spec {
        package: String,
        spec: String,
        detail: String,
    },
    #[error("package {package:?}: required package {name:?} is absent for {spec:?}")]
    Missing {
        package: String,
        spec: String,
        name: String,
    },
    #[error("package {package:?}: {spec:?} is not satisfied by selected {name}={version}={build}")]
    Unsatisfied {
        package: String,
        spec: String,
        name: String,
        version: String,
        build: String,
    },
    #[error("declared system requirement {name:?}: {detail}")]
    System { name: String, detail: String },
    #[error("invalid Conda closure: {0}")]
    Closure(String),
}

#[derive(Clone, Copy)]
enum Requirement {
    Dependency,
    Constraint,
}

/// `facts` is the complete measured name/version/build set. Frozen values used
/// during solving are not host facts and must not be passed as substitutes.
pub fn evaluate_requirements(
    packages: &[LockedCondaPackage],
    facts: &[LockedVirtualPackage],
) -> Result<(), VirtualConstraintError> {
    evaluate(packages, facts, Knowledge::Host, None).map(|_| ())
}

/// An OCI filesystem can establish its libc and Unix userspace, not its future
/// kernel, CPU or GPU driver. Return those unresolved requirements explicitly;
/// an unknown conditional is never treated as false or as a compatible host.
pub fn evaluate_image_constraints(
    packages: &[LockedCondaPackage],
    facts: &[LockedVirtualPackage],
) -> Result<Vec<String>, VirtualConstraintError> {
    evaluate(packages, facts, Knowledge::Image, None)
}

/// Admit a frozen record set without mistaking historical solve facts for
/// complete host measurements. Missing virtual dependencies remain runtime
/// requirements; a conditional needing an unavailable fact is undecidable.
/// Supplying roots also proves that no unrelated records broaden the closure.
pub fn validate_frozen_environment(
    environment: &LockedCondaEnvironment,
    roots: Option<&[String]>,
) -> Result<(), VirtualConstraintError> {
    validate_structure(environment)?;
    evaluate(
        &environment.packages,
        &environment.virtual_packages,
        Knowledge::Recorded,
        roots,
    )
    .map(|_| ())
}

/// Admit a native declaration into the existing portable lock policy.
pub fn declared_system_requirements(
    declared: Option<&gripsack_ir::workspace_model::CondaSystemRequirements>,
    platform: &str,
) -> Result<LockedCondaSystemRequirements, VirtualConstraintError> {
    let Some(declared) = declared else {
        return Ok(LockedCondaSystemRequirements::default());
    };
    if !matches!(platform, "linux-64" | "linux-aarch64") {
        return Err(VirtualConstraintError::Closure(
            "native Conda system requirements support only Linux".into(),
        ));
    }
    declared
        .locked()
        .map_err(|detail| VirtualConstraintError::Closure(detail.into()))
}

/// Replace only explicitly declared capabilities in measured solve facts.
/// These values are solver assumptions, never measurements of the runtime host.
pub fn apply_solve_baseline(
    requirements: &LockedCondaSystemRequirements,
    facts: &mut [LockedVirtualPackage],
) -> Result<(), VirtualConstraintError> {
    measured(facts)?;
    for required in &requirements.virtual_packages {
        let fact = facts
            .iter_mut()
            .find(|fact| fact.name == required.name)
            .ok_or_else(|| VirtualConstraintError::System {
                name: required.name.clone(),
                detail: "no measured capability to override for this solve target".into(),
            })?;
        fact.version.clone_from(&required.minimum_version);
    }
    evaluate_system_requirements(requirements, facts)
}

/// Native baseline facts must encode the exact declared versions. A higher
/// fact would silently solve for the updater instead of the declared floor.
pub fn validate_solve_baseline(
    requirements: &LockedCondaSystemRequirements,
    facts: &[LockedVirtualPackage],
) -> Result<(), VirtualConstraintError> {
    evaluate_system_requirements(requirements, facts)?;
    for required in &requirements.virtual_packages {
        if !facts
            .iter()
            .any(|fact| fact.name == required.name && fact.version == required.minimum_version)
        {
            return Err(VirtualConstraintError::Closure(
                "solve facts differ from the exact declared system baseline".into(),
            ));
        }
    }
    Ok(())
}

/// Bind a coherent source's frozen roots and channel policy before acquisition.
pub fn validate_frozen_request(
    environment: &LockedCondaEnvironment,
    roots: &[String],
    channels: &[String],
    requirements: &LockedCondaSystemRequirements,
) -> Result<(), VirtualConstraintError> {
    declared_policy(environment, channels)?;
    if &environment.system_requirements != requirements {
        return Err(VirtualConstraintError::Closure(
            "frozen closure changed declared system requirements; update the lock".into(),
        ));
    }
    validate_solve_baseline(requirements, &environment.virtual_packages)?;
    validate_frozen_environment(environment, Some(roots))
}

/// Independently bind an advisory solve response to its declared request.
/// This proves satisfaction and closure, not optimality or repodata-wide
/// strict-priority solver correctness (which needs the candidate universe).
pub fn validate_solved_environment(
    environment: &LockedCondaEnvironment,
    roots: &[String],
    channels: &[String],
    facts: &[LockedVirtualPackage],
    requirements: &LockedCondaSystemRequirements,
) -> Result<(), VirtualConstraintError> {
    validate_structure(environment)?;
    declared_policy(environment, channels)?;
    measured(facts)?;
    if facts != environment.virtual_packages.as_slice()
        || &environment.system_requirements != requirements
    {
        return Err(VirtualConstraintError::Closure(
            "helper changed solve facts or declared system requirements".into(),
        ));
    }
    validate_solve_baseline(requirements, facts)?;
    evaluate(&environment.packages, facts, Knowledge::Host, Some(roots)).map(|_| ())
}

fn declared_policy(
    environment: &LockedCondaEnvironment,
    channels: &[String],
) -> Result<(), VirtualConstraintError> {
    let requested = channels
        .iter()
        .map(|channel| canonical_channel(channel))
        .collect::<Result<Vec<_>, _>>()?;
    let returned = environment
        .channels
        .iter()
        .map(|channel| canonical_channel(channel))
        .collect::<Result<Vec<_>, _>>()?;
    if requested != returned || environment.channel_priority != ChannelPriority::Strict {
        return Err(VirtualConstraintError::Closure(
            "closure changed declared channel order or strict-priority policy".into(),
        ));
    }
    Ok(())
}

fn canonical_channel(raw: &str) -> Result<String, VirtualConstraintError> {
    let fail = |detail: String| VirtualConstraintError::Closure(detail);
    let canonical = crate::canonicalize_channel(raw).map_err(|error| fail(error.to_string()))?;
    let url = url::Url::parse(&canonical).map_err(|error| fail(error.to_string()))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(fail(
            "channel must be anonymous HTTPS without query or fragment".into(),
        ));
    }
    Ok(url.as_str().trim_end_matches('/').into())
}

fn validate_structure(environment: &LockedCondaEnvironment) -> Result<(), VirtualConstraintError> {
    let fail = |detail: String| VirtualConstraintError::Closure(detail);
    let (os, arch) = match environment.platform.as_str() {
        "linux-64" => ("linux", "x86_64"),
        "linux-aarch64" => ("linux", "aarch64"),
        "osx-64" => ("osx", "x86_64"),
        "osx-arm64" => ("osx", "arm64"),
        target => return Err(fail(format!("unsupported target subdir {target:?}"))),
    };
    let channels = environment
        .channels
        .iter()
        .map(|channel| canonical_channel(channel))
        .collect::<Result<BTreeSet<_>, _>>()?;
    if channels.len() != environment.channels.len() {
        return Err(fail("duplicate canonical channels".into()));
    }
    for package in &environment.packages {
        if package.subdir != "noarch"
            && (package.subdir != environment.platform
                || package.platform.as_deref().is_some_and(|value| value != os)
                || package.arch.as_deref().is_some_and(|value| value != arch))
        {
            return Err(fail(format!(
                "package {:?} has a foreign target/subdir",
                package.name
            )));
        }
        let channel = canonical_channel(&package.channel)?;
        let url = url::Url::parse(&package.url).map_err(|error| fail(error.to_string()))?;
        if !channels.contains(&channel)
            || !url
                .as_str()
                .starts_with(&format!("{channel}/{}/", package.subdir))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(fail(format!(
                "package {:?} artifact is outside the declared channel/subdir",
                package.name
            )));
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Knowledge {
    Host,
    Image,
    Recorded,
}
impl Knowledge {
    fn external(self, name: &str) -> bool {
        matches!(self, Self::Recorded)
            || matches!(self, Self::Image)
                && !matches!(name, "__unix" | "__glibc" | "__osx" | "__win")
    }
}

fn measured(
    facts: &[LockedVirtualPackage],
) -> Result<Vec<GenericVirtualPackage>, VirtualConstraintError> {
    let mut names = BTreeSet::new();
    let mut virtuals = Vec::with_capacity(facts.len());
    for fact in facts {
        let fail = |detail: String| VirtualConstraintError::Fact {
            name: fact.name.clone(),
            detail,
        };
        let name = PackageName::from_str(&fact.name).map_err(|error| fail(error.to_string()))?;
        if !name.as_normalized().starts_with("__") || !names.insert(name.clone()) {
            return Err(fail("not a unique canonical virtual-package name".into()));
        }
        virtuals.push(GenericVirtualPackage {
            name,
            version: Version::from_str(&fact.version).map_err(|error| fail(error.to_string()))?,
            build_string: fact.build.clone(),
        });
    }
    Ok(virtuals)
}

fn parse_spec(package: &str, text: &str) -> Result<MatchSpec, VirtualConstraintError> {
    // A lexical upper bound, not another MatchSpec grammar. Bound nesting
    // before the canonical parser/conditional walker can consume stack space.
    let mut depth = 0usize;
    if text.len() > 4096 {
        return Err(invalid_spec(
            package,
            text,
            "MatchSpec exceeds 4096 bytes".into(),
        ));
    }
    for byte in text.bytes() {
        match byte {
            b'[' | b'(' | b'{' => {
                depth += 1;
                if depth > 64 {
                    return Err(invalid_spec(
                        package,
                        text,
                        "MatchSpec exceeds 64 nested delimiters".into(),
                    ));
                }
            }
            b']' | b')' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    MatchSpec::from_str(
        text,
        ParseMatchSpecOptions::from(ParseStrictness::Lenient)
            .with_conditionals(true)
            .with_extras(true)
            .with_flags(true),
    )
    .map_err(|error| invalid_spec(package, text, error.to_string()))
}

fn evaluate(
    packages: &[LockedCondaPackage],
    facts: &[LockedVirtualPackage],
    knowledge: Knowledge,
    roots: Option<&[String]>,
) -> Result<Vec<String>, VirtualConstraintError> {
    let virtuals = measured(facts)?;
    if let Some(fact) = virtuals.iter().find(|fact| {
        matches!(knowledge, Knowledge::Image)
            && !matches!(fact.name.as_normalized(), "__unix" | "__glibc")
    }) {
        return Err(VirtualConstraintError::Fact {
            name: fact.name.as_normalized().into(),
            detail: "not an admitted Linux image userspace capability".into(),
        });
    }
    let environment = packages
        .iter()
        .map(records::repository_record)
        .collect::<Result<Vec<_>, _>>()?;
    let mut names = BTreeMap::new();
    for (index, record) in environment.iter().enumerate() {
        let name = record.package_record.name.as_normalized();
        if name.starts_with("__") || names.insert(name, index).is_some() {
            return Err(VirtualConstraintError::Closure(format!(
                "duplicate or virtual ordinary package name {name:?}"
            )));
        }
    }
    let mut pending = VecDeque::new();
    let mut reached = BTreeSet::new();
    let mut extras = BTreeSet::new();
    let enqueue = |index: usize, pending: &mut VecDeque<_>| -> Result<(), VirtualConstraintError> {
        let package = &packages[index];
        for (requirements, kind) in [
            (&package.depends, Requirement::Dependency),
            (&package.constrains, Requirement::Constraint),
        ] {
            for text in requirements {
                pending.push_back((
                    package.name.as_str(),
                    text.as_str(),
                    parse_spec(&package.name, text)?,
                    kind,
                ));
            }
        }
        Ok(())
    };
    if let Some(roots) = roots {
        for text in roots {
            pending.push_back((
                "declared roots",
                text.as_str(),
                parse_spec("declared roots", text)?,
                Requirement::Dependency,
            ));
        }
    } else {
        for index in 0..packages.len() {
            reached.insert(index);
            enqueue(index, &mut pending)?;
        }
    }
    let mut deferred = Vec::new();
    while let Some((package, text, spec, kind)) = pending.pop_front() {
        let name = spec.name.as_exact().ok_or_else(|| {
            invalid_spec(
                package,
                text,
                "requirements need an exact package name".into(),
            )
        })?;
        if let Some(condition) = &spec.condition {
            match condition_holds(condition, &environment, &virtuals, knowledge) {
                Some(true) => {}
                Some(false) => continue,
                None if matches!(knowledge, Knowledge::Recorded) => {
                    return Err(invalid_spec(
                        package,
                        text,
                        "conditional needs unavailable lock-recorded virtual facts".into(),
                    ));
                }
                None => {
                    deferred.push(deferred_requirement(package, text, kind));
                    continue;
                }
            }
        }
        if name.as_normalized().starts_with("__") {
            let fact = virtuals.iter().find(|fact| &fact.name == name);
            if knowledge.external(name.as_normalized())
                && (matches!(knowledge, Knowledge::Image) || fact.is_none())
            {
                deferred.push(deferred_requirement(package, text, kind));
                continue;
            }
            let Some(fact) = fact else {
                if matches!(kind, Requirement::Constraint) {
                    continue;
                }
                return Err(VirtualConstraintError::Missing {
                    package: package.into(),
                    spec: text.into(),
                    name: name.as_normalized().into(),
                });
            };
            if !spec.matches(fact) {
                return Err(VirtualConstraintError::Unsatisfied {
                    package: package.into(),
                    spec: text.into(),
                    name: name.as_normalized().into(),
                    version: fact.version.to_string(),
                    build: fact.build_string.clone(),
                });
            }
            continue;
        }
        let Some(&index) = names.get(name.as_normalized()) else {
            if matches!(kind, Requirement::Constraint) {
                continue;
            }
            return Err(VirtualConstraintError::Missing {
                package: package.into(),
                spec: text.into(),
                name: name.as_normalized().into(),
            });
        };
        let record = &environment[index];
        if !records::matches_repository(&spec, record) {
            return Err(VirtualConstraintError::Unsatisfied {
                package: package.into(),
                spec: text.into(),
                name: name.as_normalized().into(),
                version: record.package_record.version.to_string(),
                build: record.package_record.build.clone(),
            });
        }
        if matches!(kind, Requirement::Constraint) {
            continue;
        }
        if reached.insert(index) {
            enqueue(index, &mut pending)?;
        }
        for extra in spec.extras.iter().flatten() {
            if !extras.insert((index, extra.clone())) {
                continue;
            }
            let dependencies = packages[index].extra_depends.get(extra).ok_or_else(|| {
                invalid_spec(
                    package,
                    text,
                    format!("selected package has no extra {extra:?}"),
                )
            })?;
            for text in dependencies {
                pending.push_back((
                    packages[index].name.as_str(),
                    text.as_str(),
                    parse_spec(&packages[index].name, text)?,
                    Requirement::Dependency,
                ));
            }
        }
    }
    if reached.len() != packages.len() {
        let unused = packages
            .iter()
            .enumerate()
            .filter(|(index, _)| !reached.contains(index))
            .map(|(_, package)| package.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(VirtualConstraintError::Closure(format!(
            "packages outside the requested closure: {unused}"
        )));
    }
    Ok(deferred)
}
fn invalid_spec(package: &str, spec: &str, detail: String) -> VirtualConstraintError {
    VirtualConstraintError::Spec {
        package: package.into(),
        spec: spec.into(),
        detail,
    }
}
fn deferred_requirement(package: &str, text: &str, kind: Requirement) -> String {
    format!(
        "{package}: {} {text}",
        match kind {
            Requirement::Dependency => "depends",
            Requirement::Constraint => "constrains",
        }
    )
}

fn condition_holds(
    condition: &MatchSpecCondition,
    packages: &[RepoDataRecord],
    facts: &[GenericVirtualPackage],
    knowledge: Knowledge,
) -> Option<bool> {
    match condition {
        MatchSpecCondition::MatchSpec(spec) => {
            let matched = packages
                .iter()
                .any(|record| records::matches_repository(spec, record))
                || facts.iter().any(|fact| spec.matches(fact));
            let known = if matched {
                Some(true)
            } else {
                match spec.name.as_exact() {
                    Some(name)
                        if name.as_normalized().starts_with("__")
                            && knowledge.external(name.as_normalized())
                            && !facts.iter().any(|fact| &fact.name == name) =>
                    {
                        None
                    }
                    None if !matches!(knowledge, Knowledge::Host) => None,
                    _ => Some(false),
                }
            };
            match &spec.condition {
                Some(condition) => and(
                    known,
                    condition_holds(condition, packages, facts, knowledge),
                ),
                None => known,
            }
        }
        MatchSpecCondition::And(left, right) => and(
            condition_holds(left, packages, facts, knowledge),
            condition_holds(right, packages, facts, knowledge),
        ),
        MatchSpecCondition::Or(left, right) => match (
            condition_holds(left, packages, facts, knowledge),
            condition_holds(right, packages, facts, knowledge),
        ) {
            (Some(true), _) | (_, Some(true)) => Some(true),
            (Some(false), Some(false)) => Some(false),
            _ => None,
        },
    }
}
fn and(left: Option<bool>, right: Option<bool>) -> Option<bool> {
    match (left, right) {
        (Some(false), _) | (_, Some(false)) => Some(false),
        (Some(true), Some(true)) => Some(true),
        _ => None,
    }
}

/// The physical CPU's canonical archspec identity. No CONDA_OVERRIDE_* value
/// is a measurement; inability to identify the CPU remains an explicit error.
pub fn measured_architecture() -> Result<LockedVirtualPackage, VirtualConstraintError> {
    let host = archspec::cpu::host().map_err(|error| VirtualConstraintError::Fact {
        name: "__archspec".into(),
        detail: format!("{error:?}"),
    })?;
    Ok(LockedVirtualPackage {
        name: "__archspec".into(),
        version: "1".into(),
        build: host.name().into(),
    })
}

/// Explicit manifest system requirements are additional floors, not the
/// virtual-package values observed when a lock was solved. Package MatchSpecs
/// (including upper bounds and conditionals) must also pass admission.
pub fn evaluate_system_requirements(
    requirements: &LockedCondaSystemRequirements,
    facts: &[LockedVirtualPackage],
) -> Result<(), VirtualConstraintError> {
    for required in &requirements.virtual_packages {
        let fail = |detail: String| VirtualConstraintError::System {
            name: required.name.clone(),
            detail,
        };
        let measured = facts
            .iter()
            .find(|fact| fact.name == required.name)
            .ok_or_else(|| fail("no measured host capability".into()))?;
        let minimum = Version::from_str(&required.minimum_version)
            .map_err(|error| fail(error.to_string()))?;
        let available =
            Version::from_str(&measured.version).map_err(|error| fail(error.to_string()))?;
        if available < minimum
            || required
                .build
                .as_ref()
                .is_some_and(|build| build != &measured.build)
        {
            return Err(fail(format!(
                "requires >= {} build {:?}, measured {} build {:?}",
                required.minimum_version, required.build, measured.version, measured.build
            )));
        }
    }
    if let Some(required) = &requirements.archspec {
        let fail = |detail: String| VirtualConstraintError::System {
            name: "archspec".into(),
            detail,
        };
        let known = archspec::cpu::Microarchitecture::known_targets();
        let required = known
            .get(required)
            .ok_or_else(|| fail(format!("unknown required CPU {required:?}")))?;
        let measured = facts
            .iter()
            .find(|fact| fact.name == "__archspec")
            .ok_or_else(|| fail("no measured CPU capability".into()))?;
        let available = known
            .get(&measured.build)
            .ok_or_else(|| fail(format!("unknown measured CPU {:?}", measured.build)))?;
        if available.name() != required.name() && !available.is_strict_superset(required) {
            return Err(fail(format!(
                "CPU {:?} does not support {:?}",
                available.name(),
                required.name()
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
