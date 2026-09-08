//! Exact payload spelling and the legacy GitHub asset-search compatibility lane.

#[derive(Debug, thiserror::Error)]
#[error("cannot expand {pattern:?}: {reason}")]
pub struct PlaceholderError {
    pub pattern: String,
    pub reason: &'static str,
}

pub fn expand_platform(pattern: &str) -> String {
    let mut output = pattern.to_string();
    if let Some(target) = crate::host::AssetTarget::current() {
        for (placeholder, value) in target.placeholders() {
            if output.contains(placeholder) {
                output = output.replace(placeholder, &value);
            }
        }
    }
    output
}

fn versions(
    mut output: String,
    raw: Option<&str>,
    bare_source: Option<&str>,
) -> Result<String, PlaceholderError> {
    if output.contains("{version.bare}") {
        let version = bare_source.ok_or_else(|| PlaceholderError {
            pattern: output.clone(),
            reason: "{version.bare} requires a resolved version",
        })?;
        let bare = version.strip_prefix('v').unwrap_or(version);
        if bare.is_empty() {
            return Err(PlaceholderError {
                pattern: output,
                reason: "{version.bare} would be empty",
            });
        }
        output = output.replace("{version.bare}", bare);
    }
    if output.contains("{version}") {
        let version = raw.ok_or_else(|| PlaceholderError {
            pattern: output.clone(),
            reason: "{version} requires a resolved version",
        })?;
        output = output.replace("{version}", version);
    }
    Ok(output)
}

pub fn expand(pattern: &str, version: Option<&str>) -> Result<String, PlaceholderError> {
    if version == Some("") && pattern.contains("{version}") {
        return Err(PlaceholderError {
            pattern: pattern.into(),
            reason: "{version} would be empty",
        });
    }
    let expanded = versions(expand_platform(pattern), version, version)?;
    if expanded.contains(['{', '}']) {
        return Err(PlaceholderError {
            pattern: pattern.into(),
            reason: "unresolved or unsupported placeholder",
        });
    }
    Ok(expanded)
}

pub fn payload_path(pattern: &str, version: Option<&str>) -> Result<String, PlaceholderError> {
    let output = expand(pattern, version)?;
    if !output.is_empty()
        && (output.starts_with(['/', '~'])
            || output.contains(['\\', '\0'])
            || output
                .split('/')
                .any(|part| matches!(part, "" | "." | "..")))
    {
        return Err(PlaceholderError {
            pattern: pattern.into(),
            reason: "expanded path is not a safe payload-relative path",
        });
    }
    Ok(output)
}

/// Raw {version} keeps its historical raw-first search. Bare never changes lanes.
pub fn asset_patterns(pattern: &str, tag: &str) -> Result<Vec<String>, PlaceholderError> {
    let platform = expand_platform(pattern);
    if !platform.contains("{version}") || !tag.starts_with('v') {
        return Ok(vec![versions(platform, Some(tag), Some(tag))?]);
    }
    let first = versions(platform.clone(), Some(tag), Some(tag))?;
    let bare = tag.strip_prefix('v').unwrap_or(tag);
    let second = versions(platform, Some(bare), Some(tag))?;
    if first == second {
        Ok(vec![first])
    } else {
        Ok(vec![first, second])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_bare_version_is_shared_without_changing_legacy_asset_precedence() {
        for (tag, expected) in [
            ("v0.12.1", "0.12.1"),
            ("0.12.1", "0.12.1"),
            ("vv1", "v1"),
            ("V1", "V1"),
            ("release-1", "release-1"),
        ] {
            assert_eq!(
                payload_path("pkg-{version.bare}/bin", Some(tag)).unwrap(),
                format!("pkg-{expected}/bin")
            );
            assert_eq!(
                asset_patterns("pkg-{version.bare}.tar", tag).unwrap(),
                vec![format!("pkg-{expected}.tar")]
            );
            assert_eq!(
                payload_path("pkg-{version}/bin", Some(tag)).unwrap(),
                format!("pkg-{tag}/bin")
            );
        }
        assert_eq!(
            asset_patterns("{version}-{version.bare}", "v1").unwrap(),
            vec!["v1-1", "1-1"]
        );
        assert_eq!(asset_patterns("{version}", "1").unwrap(), vec!["1"]);
    }

    #[test]
    fn missing_empty_or_escaping_expansions_never_become_paths() {
        assert!(payload_path("{version.bare}/bin", None).is_err());
        assert!(payload_path("{version.bare}/bin", Some("v")).is_err());
        for value in [
            "../escape",
            "/absolute",
            "..",
            "~",
            "bad\\path",
            "bad\0path",
        ] {
            assert!(payload_path("{version.bare}", Some(value)).is_err());
        }
        assert!(payload_path("{unknown}/bin", Some("v1")).is_err());
        assert_eq!(payload_path("", None).unwrap(), "");
        assert_eq!(payload_path("literal", None).unwrap(), "literal");
    }
}
