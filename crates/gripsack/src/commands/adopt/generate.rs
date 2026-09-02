//! Generation: the repo artifacts adopt writes — payload copy, the
//! module source, the host-entrypoint edit. Pure functions at the
//! edges (everything string-shaped is unit-tested); the two io
//! functions are the only side effects.

use std::path::{Path, PathBuf};

/// Module name from the adopted path: dir basename, or file name with
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
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if clean.is_empty() { None } else { Some(clean) }
}

/// A TS-safe identifier for the module name.
pub fn ident(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
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

/// The generated modules/<name>.ts — exactly what the user would have
/// written by hand.
pub fn module_source(name: &str, to: &str, first_rel: &str, mode: &str, is_dir: bool) -> String {
    match (mode, is_dir) {
        (MODE_MERGE, _) => format!(
            "import {{ merge, module }} from \"@gripsack/core\";\n\n\
             export default module(\"{name}\", {{\n  config: {{\n    \"configs/{name}/{first_rel}\": merge(\"{to}\", \"#\"),\n  }},\n}});\n"
        ),
        (_, true) => format!(
            "import {{ module, tree }} from \"@gripsack/core\";\n\n\
             export default module(\"{name}\", {{\n  config: tree(\"configs/{name}\", \"{to}\", \"{mode}\"),\n}});\n"
        ),
        _ => {
            let ctor = match mode {
                MODE_OWNED => "symlink",
                _ => "trackedCopy",
            };
            format!(
                "import {{ {ctor}, module }} from \"@gripsack/core\";\n\n\
                 export default module(\"{name}\", {{\n  config: {{\n    \"configs/{name}/{first_rel}\": {ctor}(\"{to}\"),\n  }},\n}});\n"
            )
        }
    }
}

/// Insert the import and the modules-array entry into a defineEnv
/// host file. Conservative: returns None on any shape it doesn't
/// recognize — the caller shows the snippet instead of risking a
/// mangled host file.
pub fn update_host(src: &str, name: &str) -> Option<String> {
    let ident = ident(name);
    let import = format!("import {ident} from \"../modules/{name}.ts\";");
    if src.contains(&import) {
        // already imported: still ensure the array entry
        return insert_module_entry(src, &ident);
    }
    let mut lines: Vec<String> = src.lines().map(str::to_string).collect();
    let last_import = lines
        .iter()
        .rposition(|l| l.trim_start().starts_with("import ") && l.contains(" from "))?;
    lines.insert(last_import + 1, import);
    let joined = lines.join("\n") + "\n";
    insert_module_entry(&joined, &ident)
}

fn insert_module_entry(src: &str, ident: &str) -> Option<String> {
    let mut lines: Vec<String> = src.lines().map(str::to_string).collect();
    // the LAST match — the init template documents a `modules: [`
    // example inside its header comment; the first match inserts into
    // the comment and the dangling comma breaks eval
    let pos = lines
        .iter()
        .rposition(|l| l.contains("modules: [") && !l.trim_start().starts_with("//"))?;
    if lines[pos].contains(']') {
        // single-line array — insert before the closing bracket
        let close = lines[pos].rfind(']')?;
        let inner = &lines[pos][..close];
        if inner.split([',', '['].as_ref()).any(|t| t.trim() == ident) {
            return Some(src.to_string()); // already listed
        }
        let insertion = if inner.trim_end().ends_with('[') {
            ident.to_string() // empty array: `[]` → `[ident]`
        } else {
            format!(", {ident}") // non-empty: `[a]` → `[a, ident]`
        };
        lines[pos].insert_str(close, &insertion);
    } else {
        // multiline array: bail if already listed, else insert indented
        if lines[pos..]
            .iter()
            .take_while(|l| !l.contains(']'))
            .any(|l| l.trim().trim_end_matches(',') == ident)
        {
            return Some(src.to_string());
        }
        let indent: String = lines[pos]
            .chars()
            .take_while(|c| c.is_whitespace())
            .collect();
        lines.insert(pos + 1, format!("{indent}  {ident},"));
    }
    Some(lines.join("\n") + "\n")
}

pub fn host_snippet(name: &str) -> String {
    let ident = ident(name);
    format!(
        "import {ident} from \"../modules/{name}.ts\";\n// and add `{ident}` to the modules array"
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

    const HOST_MULTILINE: &str = "import { defineEnv } from \"@gripsack/core\";\nimport hello from \"../modules/hello.ts\";\n\nexport default defineEnv(() => ({\n  tags: [],\n  modules: [\n    hello,\n  ],\n}));\n";

    #[test]
    fn update_host_multiline_array() {
        let out = update_host(HOST_MULTILINE, "zed").unwrap();
        assert!(out.contains("import zed from \"../modules/zed.ts\";"));
        assert!(out.contains("  zed,\n"), "entry inserted in array:\n{out}");
        // idempotent
        let twice = update_host(&out, "zed").unwrap();
        assert_eq!(twice.matches("modules/zed").count(), 1);
        assert_eq!(twice.matches("zed,").count(), 1);
    }

    #[test]
    fn update_host_single_line_arrays() {
        let empty = "import { defineEnv } from \"@gripsack/core\";\nexport default defineEnv(() => ({ modules: [] }));\n";
        let out = update_host(empty, "helix").unwrap();
        assert!(out.contains("modules: [helix]"), "{out}");

        let one = "import { defineEnv } from \"@gripsack/core\";\nexport default defineEnv(() => ({ modules: [hello] }));\n";
        let out = update_host(one, "helix").unwrap();
        assert!(
            out.contains("modules: [hello, helix]") || out.contains("helix, ]"),
            "{out}"
        );
    }

    #[test]
    fn update_host_ignores_the_template_header_comment() {
        // the init template documents a `modules: [` example inside
        // its // header comment — inserting there broke eval with a
        // dangling comma (the comment's example swallowed the import)
        let template = include_str!("../../../template/env-repo/hosts/myhost.ts");
        let out = update_host(template, "zed").unwrap();
        assert!(
            out.contains("modules: [hello, zed]"),
            "entry lands in the real array, not the comment:\n{out}"
        );
        // and the comment example is untouched
        assert!(out.contains("//       hello,"));
    }

    #[test]
    fn update_host_bails_on_unexpected_shape() {
        assert!(update_host("const x = 1;\n", "zed").is_none());
        assert!(update_host("import a from \"./a.ts\";\n", "zed").is_none());
    }

    #[test]
    fn module_source_shapes() {
        let dir = module_source("helix", "~/.config/helix", "config.toml", MODE_OWNED, true);
        assert!(dir.contains("tree(\"configs/helix\", \"~/.config/helix\", \"owned\")"));
        let file = module_source(
            "gitconfig",
            "~/.gitconfig",
            "gitconfig",
            MODE_TRACKED_COPY,
            false,
        );
        assert!(file.contains("trackedCopy(\"~/.gitconfig\")"));
        let merge = module_source("bashrc", "~/.bashrc", "bashrc", MODE_MERGE, false);
        assert!(merge.contains("merge(\"~/.bashrc\", \"#\")"));
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
