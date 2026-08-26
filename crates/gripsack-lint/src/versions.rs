//! Module → pinned tool version, from the host lockfile (0011 §3).

use std::collections::BTreeMap;
use std::path::Path;

/// Module → pinned tool version, from the host lockfile (0011 §3).
pub(crate) fn tool_versions(repo: &Path, host: Option<&str>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Some(host) = host else { return out };
    let Ok(text) = std::fs::read_to_string(repo.join("locks").join(format!("{host}.lock"))) else {
        return out;
    };
    let Ok(data) = serde_json::from_str::<serde_json::Value>(&text) else {
        return out;
    };
    if let Some(modules) = data.get("modules").and_then(|m| m.as_object()) {
        for (name, entry) in modules {
            if let Some(version) = entry
                .get("resolved")
                .and_then(|r| r.get("version"))
                .and_then(|v| v.as_str())
            {
                out.insert(name.clone(), version.to_string());
            }
        }
    }
    out
}
