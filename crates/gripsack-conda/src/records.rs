//! One conversion from the strict portable wire to canonical Rattler records.
//! No JSON round-trip buffer and no second version/build matcher are involved.
use gripsack_ir::workspace_model::lock::{LockedCondaPackage, LockedNoArch};
use rattler_conda_types::{
    NoArchType, PackageName, PackageRecord, VersionWithSource, package::RunExportsJson,
    utils::TimestampMs,
};
use rattler_digest::{Md5, Sha256, parse_digest_from_hex};
use std::str::FromStr;

#[derive(Debug, thiserror::Error)]
#[error("package {package:?}: invalid {field}: {detail}")]
pub struct RecordError {
    pub package: String,
    pub field: &'static str,
    pub detail: String,
}
fn invalid(
    package: &LockedCondaPackage,
    field: &'static str,
    detail: impl std::fmt::Display,
) -> RecordError {
    RecordError {
        package: package.name.clone(),
        field,
        detail: detail.to_string(),
    }
}
fn timestamp(
    package: &LockedCondaPackage,
    field: &'static str,
    value: Option<u64>,
) -> Result<Option<TimestampMs>, RecordError> {
    value
        .map(|value| {
            let value = i64::try_from(value).map_err(|error| invalid(package, field, error))?;
            let value = jiff::Timestamp::from_millisecond(value)
                .map_err(|error| invalid(package, field, error))?;
            Ok(TimestampMs::from_timestamp_millis(value))
        })
        .transpose()
}
pub fn package_record(package: &LockedCondaPackage) -> Result<PackageRecord, RecordError> {
    let name =
        PackageName::from_str(&package.name).map_err(|error| invalid(package, "name", error))?;
    let version = VersionWithSource::from_str(&package.version)
        .map_err(|error| invalid(package, "version", error))?;
    let mut record = PackageRecord::new(name, version, package.build.clone());
    record.build_number = package.build_number;
    record.subdir = package.subdir.clone();
    record.arch = package.arch.clone();
    record.platform = package.platform.clone();
    record.noarch = match package.noarch {
        LockedNoArch::None => NoArchType::none(),
        LockedNoArch::Generic => NoArchType::generic(),
        LockedNoArch::Python => NoArchType::python(),
    };
    record.depends = package.depends.clone();
    record.constrains = package.constrains.clone();
    record.extra_depends = package.extra_depends.clone();
    record.features = package.features.clone();
    record.flags = package
        .flags
        .iter()
        .map(|flag| {
            flag.parse()
                .map_err(|error| invalid(package, "flag", error))
        })
        .collect::<Result<_, _>>()?;
    record.track_features = package.track_features.clone();
    record.python_site_packages_path = package.python_site_packages_path.clone();
    record.run_exports = package.run_exports.as_ref().map(|exports| RunExportsJson {
        weak: exports.weak.clone(),
        strong: exports.strong.clone(),
        noarch: exports.noarch.clone(),
        weak_constrains: exports.weak_constrains.clone(),
        strong_constrains: exports.strong_constrains.clone(),
    });
    record.license = package.license.clone();
    record.license_family = package.license_family.clone();
    record.size = package.size;
    record.legacy_bz2_size = package.legacy_bz2_size;
    record.timestamp = timestamp(package, "timestamp", package.timestamp)?;
    record.indexed_timestamp = timestamp(package, "indexed_timestamp", package.indexed_timestamp)?;
    record.sha256 = Some(
        parse_digest_from_hex::<Sha256>(&package.sha256)
            .ok_or_else(|| invalid(package, "sha256", "invalid digest"))?,
    );
    record.md5 = package
        .md5
        .as_deref()
        .map(|value| {
            parse_digest_from_hex::<Md5>(value)
                .ok_or_else(|| invalid(package, "md5", "invalid digest"))
        })
        .transpose()?;
    record.legacy_bz2_md5 = package
        .legacy_bz2_md5
        .as_deref()
        .map(|value| {
            parse_digest_from_hex::<Md5>(value)
                .ok_or_else(|| invalid(package, "legacy_bz2_md5", "invalid digest"))
        })
        .transpose()?;
    record.attestations_sha256 = package
        .attestations_sha256
        .as_deref()
        .map(|value| {
            parse_digest_from_hex::<Sha256>(value)
                .ok_or_else(|| invalid(package, "attestations_sha256", "invalid digest"))
        })
        .transpose()?;
    record.purls = package
        .purls
        .as_ref()
        .map(|purls| {
            purls
                .iter()
                .map(|value| {
                    value
                        .parse()
                        .map_err(|error| invalid(package, "purl", error))
                })
                .collect::<Result<_, _>>()
        })
        .transpose()?;
    Ok(record)
}

pub fn repository_record(
    package: &LockedCondaPackage,
) -> Result<rattler_conda_types::RepoDataRecord, RecordError> {
    let url = url::Url::parse(&package.url).map_err(|error| invalid(package, "url", error))?;
    let identifier = rattler_conda_types::package::DistArchiveIdentifier::try_from_url(&url)
        .ok_or_else(|| {
            invalid(
                package,
                "url",
                "artifact URL has no canonical archive identifier",
            )
        })?;
    Ok(rattler_conda_types::RepoDataRecord {
        package_record: package_record(package)?,
        identifier,
        url,
        channel: Some(package.channel.clone()),
    })
}

/// Rattler's `Matches<RepoDataRecord>` checks the canonical package selectors
/// and artifact URL, but deliberately leaves repository location selectors to
/// the solver. Frozen admission must bind those selectors too.
pub fn matches_repository(
    spec: &rattler_conda_types::MatchSpec,
    record: &rattler_conda_types::RepoDataRecord,
) -> bool {
    use rattler_conda_types::Matches;
    spec.matches(record)
        && spec
            .subdir
            .as_ref()
            .is_none_or(|subdir| subdir == &record.package_record.subdir)
        && spec.file_name.as_ref().is_none_or(|name| {
            record
                .url
                .path_segments()
                .and_then(|mut parts| parts.next_back())
                == Some(name.as_str())
        })
        && spec.channel.as_ref().is_none_or(|channel| {
            record.channel.as_deref().is_some_and(|actual| {
                actual.trim_end_matches('/') == channel.base_url.as_str().trim_end_matches('/')
            }) && channel.platforms.as_ref().is_none_or(|platforms| {
                platforms
                    .iter()
                    .any(|platform| platform.as_str() == record.package_record.subdir)
            })
        })
}
