//! Generate explicit workspace profile files and a conservative root import.

use std::path::{Path, PathBuf};

/// Profile name from the adopted path: dir basename, or file name with
/// the dotfile dot stripped. Sanitized to [alnum - _].
pub fn default_name(dest: &Path, is_dir: bool) -> Option<String> {
    let raw = if is_dir {
        dest.file_name()?.to_string_lossy().into_owned()
    } else {
        dest.file_name()?
            .to_string_lossy()
            .trim_start_matches('.')
            .to_string()
    };
    let clean: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if clean.is_empty() { None } else { Some(clean) }
}

/// A TS-safe identifier for the profile name: [alnum _], never
/// digit-leading, never empty (0033 R6).
pub fn ident(name: &str) -> String {
    let mapped: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    match mapped.chars().next() {
        Some(c) if c.is_ascii_digit() => format!("_{mapped}"),
        Some(_) => mapped,
        None => "_".into(),
    }
}

/// A TS string literal for an interpolated value (0033 R6): JSON
/// quoting IS valid TypeScript, and a path containing quotes or
/// backslashes can neither break nor inject the generated profile.
fn quoted(value: &str) -> String {
    serde_json::to_string(value).expect("a string always serializes")
}

/// "~/…" for a path under $HOME, absolute otherwise.
pub fn tilde(dest: &Path) -> String {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    match dest.strip_prefix(&home) {
        Ok(rest) => format!("~/{}", rest.to_string_lossy()),
        Err(_) => dest.to_string_lossy().into_owned(),
    }
}

pub const MODE_OWNED: &str = "owned";
pub const MODE_TRACKED_COPY: &str = "tracked_copy";
pub const MODE_MERGE: &str = "merge";

/// Every inventoried file gets an explicit origin and ownership policy.
pub fn profile_source(name: &str, to: &str, files: &[String], mode: &str, is_dir: bool) -> String {
    let constructor = match mode {
        MODE_OWNED => "symlinkTo",
        MODE_MERGE => "managedBlock",
        _ => "trackedCopyTo",
    };
    let mut source = format!(
        "import {{ profile, file, repoFile, identity, {constructor} }} from \"@gripsack/core\";\n\nexport default profile({}, {{\n  files: [\n",
        quoted(name),
    );
    for relative in files {
        let origin = quoted(&format!("configs/{name}/{relative}"));
        let destination = quoted(&if is_dir {
            format!("{}/{relative}", to.trim_end_matches('/'))
        } else {
            to.to_owned()
        });
        let marker = if mode == MODE_MERGE { ", \"#\"" } else { "" };
        source.push_str(&format!(
            "    file({{ source: repoFile({origin}), content: identity(), destination: {constructor}({destination}{marker}) }}),\n",
        ));
    }
    source.push_str("  ],\n});\n");
    source
}

/// Only edit the simple multiline workspace form emitted by init.
/// Other authoring styles receive a snippet for explicit manual integration.
pub fn update_workspace(src: &str, name: &str) -> Option<String> {
    let ident = format!("adopted_{}", ident(name));
    let import = format!(
        "import {ident} from {};",
        quoted(&format!("./profiles/{name}.ts"))
    );
    if src.contains(&import) {
        return None;
    }
    if src.contains("/*") || src.contains('`') || src.contains(&ident) {
        return None;
    }
    let mut lines: Vec<String> = src.lines().map(str::to_owned).collect();
    let opening = lines
        .iter()
        .position(|line| line == "export default defineWorkspace(() => workspace({")?;
    if lines.get(opening + 1)? != "  outputs: [" {
        return None;
    }
    lines.insert(opening + 2, format!("    {ident},"));
    lines.insert(0, import);
    Some(lines.join("\n") + "\n")
}

pub fn workspace_snippet(name: &str) -> String {
    let ident = format!("adopted_{}", ident(name));
    format!(
        "import {ident} from {};\n// Add `{ident}` to the workspace outputs array.",
        quoted(&format!("./profiles/{name}.ts")),
    )
}

/// Copy the live payload into configs/<name>/ — verbatim, file
/// symlinks dereferenced (adopt takes content, not indirection).
pub fn write_payload(
    repo: &Path,
    dest: &Path,
    name: &str,
    rel_files: &[String],
    is_dir: bool,
) -> std::io::Result<()> {
    let payload = repo.join("configs").join(name);
    for rel in rel_files {
        let source = if is_dir {
            dest.join(rel)
        } else {
            dest.to_path_buf()
        };
        // copy on a fifo/device blocks forever — inventory already
        // refuses them; this is the belt to that braces
        if !std::fs::metadata(&source)
            .map(|m| m.file_type().is_file())
            .unwrap_or(false)
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "{} is not a regular file — refusing to adopt it",
                    source.display()
                ),
            ));
        }
        let out = payload.join(rel);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&source, &out)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserts_profile_in_starter_workspace() {
        let template = include_str!("../../../template/env-repo/gripsack.ts");
        let out = update_workspace(template, "zed").unwrap();
        assert!(out.contains("import adopted_zed from \"./profiles/zed.ts\";"));
        assert!(out.contains("  outputs: [\n    adopted_zed,"));
        assert!(update_workspace(&out, "zed").is_none());
    }

    #[test]
    fn unfamiliar_workspace_shapes_are_left_for_review() {
        for source in ["const x = 1;\n", "export default myWorkspace;\n"] {
            assert!(update_workspace(source, "zed").is_none());
        }
    }

    #[test]
    fn ident_is_always_a_valid_ts_identifier() {
        assert_eq!(ident("9pocket"), "_9pocket");
        assert_eq!(ident("my-mod"), "my_mod");
        assert_eq!(ident("---"), "___");
        assert_eq!(ident("..."), "___");
        assert_eq!(ident(""), "_");
    }

    #[test]
    fn profile_source_escapes_hostile_strings() {
        let src = profile_source(
            "demo",
            "~/.config/we\"ird.toml",
            &["a\"b.toml".to_owned()],
            MODE_TRACKED_COPY,
            false,
        );
        assert!(
            src.contains(r#"trackedCopyTo("~/.config/we\"ird.toml")"#),
            "{src}"
        );
        assert!(
            src.contains(r#"repoFile("configs/demo/a\"b.toml")"#),
            "{src}"
        );
    }

    #[test]
    fn profile_source_preserves_file_destinations_and_ownership() {
        let files = vec!["config.toml".to_owned(), "themes/custom.toml".to_owned()];
        let dir = profile_source("helix", "~/.config/helix", &files, MODE_OWNED, true);
        assert!(dir.contains("symlinkTo(\"~/.config/helix/config.toml\")"));
        assert!(dir.contains("symlinkTo(\"~/.config/helix/themes/custom.toml\")"));
        let file = profile_source(
            "gitconfig",
            "~/.gitconfig",
            &files[..1],
            MODE_TRACKED_COPY,
            false,
        );
        assert!(file.contains("trackedCopyTo(\"~/.gitconfig\")"));
        let merge = profile_source("bashrc", "~/.bashrc", &files[..1], MODE_MERGE, false);
        assert!(merge.contains("managedBlock(\"~/.bashrc\", \"#\")"));
    }

    #[test]
    fn names_and_idents() {
        assert_eq!(
            default_name(Path::new("/home/u/.config/helix"), true).unwrap(),
            "helix"
        );
        assert_eq!(
            default_name(Path::new("/home/u/.gitconfig"), false).unwrap(),
            "gitconfig"
        );
        assert_eq!(ident("google-chrome"), "google_chrome");
    }
}
