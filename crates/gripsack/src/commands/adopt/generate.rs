//! Generate explicit workspace profile files and a conservative root import.

use super::inspect::Inventory;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
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
pub fn profile_source<'a>(
    name: &str,
    to: &str,
    files: impl IntoIterator<Item = &'a str>,
    mode: &str,
    is_dir: bool,
) -> String {
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
    // Preserve every original byte so that removing the two lines undoes the edit.
    if src.contains("/*") || src.contains('`') || src.contains(&ident) {
        return None;
    }
    let mut lines = src.split_inclusive('\n');
    let mut end = 0;
    lines.find(|line| {
        end += line.len();
        line.trim_end_matches(['\r', '\n']) == "export default defineWorkspace(() => workspace({"
    })?;
    let outputs = lines.next()?;
    let newline = outputs.strip_prefix("  outputs: [")?;
    if !matches!(newline, "\n" | "\r\n") {
        return None;
    }
    let (before, after) = src.split_at(end + outputs.len());
    Some(format!(
        "{import}{newline}{before}    {ident},{newline}{after}"
    ))
}

pub fn workspace_snippet(name: &str) -> String {
    let ident = format!("adopted_{}", ident(name));
    format!(
        "import {ident} from {};\n// Add `{ident}` to the workspace outputs array.",
        quoted(&format!("./profiles/{name}.ts")),
    )
}

/// The payload directory and profile file are ours only after exclusive
/// creation. The workspace is never ours; only its two inserted lines are.
#[derive(Default)]
pub(super) struct Writes {
    payload_files: Option<usize>,
    profile_created: bool,
    pub(super) workspace_edited: bool,
}

impl Writes {
    pub(super) fn generate(
        &mut self,
        repo: &Path,
        dest: &Path,
        name: &str,
        inv: &Inventory<'_>,
        mode: &str,
        is_dir: bool,
    ) -> Result<(), String> {
        let configs = repo.join("configs");
        let payload = configs.join(name);
        fs::create_dir_all(configs)
            .and_then(|()| fs::create_dir(&payload))
            .map_err(|e| format!("cannot create configs/{name}/: {e}"))?;
        let created = self.payload_files.insert(0);
        for file in &inv.files {
            copy_payload_file(&file.source, &payload.join(inv.selector(file)), created)
                .map_err(|e| format!("cannot copy {:?}: {e}", file.source))?;
        }

        let profiles = repo.join("profiles");
        let mut profile = fs::create_dir_all(&profiles)
            .and_then(|()| fs::File::create_new(profiles.join(format!("{name}.ts"))))
            .map_err(|e| format!("cannot create profiles/{name}.ts: {e}"))?;
        self.profile_created = true;
        let source = profile_source(
            name,
            &tilde(dest),
            inv.files.iter().map(|f| inv.selector(f)),
            mode,
            is_dir,
        );
        profile
            .write_all(source.as_bytes())
            .map_err(|e| format!("cannot write profiles/{name}.ts: {e}"))?;

        self.integrate(repo, name)
            .map_err(|e| format!("cannot integrate profile in gripsack.ts: {e}"))
    }

    fn integrate(&mut self, repo: &Path, name: &str) -> io::Result<()> {
        let root = Path::new("gripsack.ts");
        let path = repo.join(root);
        let metadata = path.symlink_metadata()?;
        // Atomic replacement would detach an alias. Leave it for manual integration.
        if !metadata.is_file() || metadata.nlink() != 1 {
            return Ok(());
        }
        let Some(updated) = update_workspace(&fs::read_to_string(&path)?, name) else {
            return Ok(());
        };
        // A writable parent alone is not permission to edit this file.
        fs::OpenOptions::new().write(true).open(&path)?;
        // Stage privately beside the original: a short write cannot truncate
        // user source or expose it with wider permissions.
        let mut staged = tempfile::NamedTempFile::new_in(repo)?;
        staged.write_all(updated.as_bytes())?;
        staged.as_file().set_permissions(metadata.permissions())?;
        staged.as_file().sync_all()?;
        staged.persist(path)?;
        self.workspace_edited = true;
        Ok(())
    }

    pub(super) fn report<'a>(
        &'a self,
        name: &'a str,
        inv: &'a Inventory<'_>,
    ) -> impl fmt::Display + 'a {
        fmt::from_fn(move |out| {
            let Some(count) = self.payload_files else {
                return writeln!(out, "  no adoption files were created");
            };
            writeln!(out, "  created by this adoption (remove to abandon):")?;
            writeln!(out, "    configs/{name}/")?;
            for file in &inv.files[..count] {
                writeln!(out, "      {}", inv.selector(file).escape_debug())?;
            }
            if self.profile_created {
                writeln!(out, "    profiles/{name}.ts")?;
            }
            if self.workspace_edited {
                let binding = format!("adopted_{}", ident(name));
                writeln!(
                    out,
                    "  edited gripsack.ts — keep this file. To undo, remove only the `import {binding} from \"./profiles/{name}.ts\";` line and the `{binding},` entry in `outputs`."
                )?;
            }
            Ok(())
        })
    }
}

fn copy_payload_file(source: &Path, dest: &Path, created: &mut usize) -> io::Result<()> {
    // Never open a fifo or device to copy its contents.
    let metadata = fs::metadata(source)?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    let mut input = fs::File::open(source)?;
    fs::create_dir_all(dest.parent().expect("payload file has a parent"))?;
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(metadata.permissions().mode())
        .open(dest)?;
    // Even a short/failed copy leaves a file to account for.
    *created += 1;
    io::copy(&mut input, &mut output)?;
    output.set_permissions(metadata.permissions())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interrupted_payload_reports_only_the_created_prefix() {
        let repo = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        fs::write(target.path().join("first"), b"taken").unwrap();
        fs::write(target.path().join("gone"), b"disappeared").unwrap();
        fs::write(target.path().join("last"), b"not reached").unwrap();
        let inv = crate::commands::adopt::inspect::inspect(
            target.path(),
            target.path().symlink_metadata().unwrap(),
        )
        .unwrap();
        fs::remove_file(target.path().join("gone")).unwrap();
        let mut writes = Writes::default();
        writes
            .generate(repo.path(), target.path(), "demo", &inv, MODE_OWNED, true)
            .unwrap_err();
        let report = writes.report("demo", &inv).to_string();
        let payload = repo.path().join("configs/demo");
        assert_eq!(fs::read(payload.join("first")).unwrap(), b"taken");
        assert!(report.contains("first"));
        for absent in ["gone", "last"] {
            assert!(!payload.join(absent).exists());
            assert!(!report.contains(absent));
        }
        assert!(!report.contains("profiles/demo.ts"));
        assert!(!report.contains("gripsack.ts"));
    }

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
            ["a\"b.toml"],
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
        let files = ["config.toml", "themes/custom.toml"];
        let dir = profile_source("helix", "~/.config/helix", files, MODE_OWNED, true);
        assert!(dir.contains("symlinkTo(\"~/.config/helix/config.toml\")"));
        assert!(dir.contains("symlinkTo(\"~/.config/helix/themes/custom.toml\")"));
        let file = profile_source(
            "gitconfig",
            "~/.gitconfig",
            [files[0]],
            MODE_TRACKED_COPY,
            false,
        );
        assert!(file.contains("trackedCopyTo(\"~/.gitconfig\")"));
        let merge = profile_source("bashrc", "~/.bashrc", [files[0]], MODE_MERGE, false);
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
