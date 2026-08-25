//! Template rendering and managed-block merging (0001 §3.7).
//!
//! Prior art, deliberately stolen and avoided (design review):
//! - conda init: replace-not-merge region semantics + the "managed by"
//!   banner; avoided its duplicate-block accumulation and its
//!   line-anchored-only markers (ours tolerate indentation).
//! - chezmoi: missingkey=error (undefined variable fails loudly),
//!   opt-in per file (our mode IS the opt-in), and its CRLF pitfall
//!   (we preserve the file's existing line endings instead of adding
//!   a directive).
//! - direnv: the anti-pattern — appending without markers. Never.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::path::Path;

use crate::ctx::ExecError;

/// Render `{{ name }}` substitutions. `{{{{` is a literal `{{` — the
/// chezmoi pitfall: payloads that themselves carry template syntax
/// (helm values, jinja configs) must be expressible. Anything else
/// (conditionals, filters) is out of scope by design: per-host logic
/// lives in the frontend, which computes `vars` at eval time.
pub fn render_template(
    bytes: &[u8],
    vars: &BTreeMap<String, String>,
    from: &str,
) -> Result<Vec<u8>, ExecError> {
    let text = std::str::from_utf8(bytes).map_err(|_| ExecError::Step {
        module: from.to_string(),
        step: "render".into(),
        detail: format!("template payload {from:?} is not UTF-8"),
    })?;
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("{{") {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        if let Some(stripped) = rest.strip_prefix("{{{{") {
            out.push_str("{{");
            rest = stripped;
            continue;
        }
        let body = &rest[2..];
        let Some(end) = body.find("}}") else {
            return Err(ExecError::Step {
                module: from.to_string(),
                step: "render".into(),
                detail: format!("unbalanced `{{{{` in template {from:?}"),
            });
        };
        let name = body[..end].trim();
        match vars.get(name) {
            Some(v) => out.push_str(v),
            None => {
                return Err(ExecError::Step {
                    module: from.to_string(),
                    step: "render".into(),
                    detail: format!(
                        "template {from:?} references undefined variable {name:?} (have: {})",
                        vars.keys().cloned().collect::<Vec<_>>().join(", ")
                    ),
                });
            }
        }
        rest = &body[end + 2..];
    }
    out.push_str(rest);
    Ok(out.into_bytes())
}

/// Comment style for a destination: (line prefix, line suffix).
/// An explicit `marker` overrides the prefix; paired styles (html
/// comments) only ever come from the table.
fn comment_style(dest: &Path, marker: Option<&str>) -> (Cow<'static, str>, &'static str) {
    if let Some(m) = marker {
        return (Cow::Owned(m.to_string()), "");
    }
    // rc files carry no extension — key on the basename first
    if let Some(base) = dest.file_name().and_then(|b| b.to_str()) {
        match base {
            ".vimrc" | "vimrc" | "init.vim" => return (Cow::Borrowed("\""), ""),
            ".bashrc" | "bashrc" | ".zshrc" | "zshrc" | ".profile" | "profile"
            | ".bash_profile" | "bash_profile" | ".gitconfig" | "gitconfig" | "tmux.conf"
            | ".tmux.conf" | "ssh_config" => return (Cow::Borrowed("#"), ""),
            _ => {}
        }
    }
    match dest
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
    {
        "js" | "ts" | "jsx" | "tsx" | "jsonc" | "css" | "scss" | "rs" | "go" | "c" | "h"
        | "cpp" | "hpp" | "java" | "kt" | "swift" => (Cow::Borrowed("//"), ""),
        "lua" | "sql" => (Cow::Borrowed("--"), ""),
        "vim" => (Cow::Borrowed("\""), ""),
        "html" | "xml" | "svg" | "md" => (Cow::Borrowed("<!--"), " -->"),
        // `#` is the lingua franca: sh, python, toml, yaml, ini-ish —
        // and the default for anything unknown
        _ => (Cow::Borrowed("#"), ""),
    }
}

/// The generated marker lines for a module's block.
pub fn block_markers(module: &str, dest: &Path, marker: Option<&str>) -> (String, String) {
    let (pre, suf) = comment_style(dest, marker);
    (
        format!("{pre} >>> gripsack module={module} >>>{suf}"),
        format!("{pre} <<< gripsack <<<{suf}"),
    )
}

fn banner(dest: &Path, marker: Option<&str>) -> String {
    let (pre, suf) = comment_style(dest, marker);
    format!("{pre} !! managed by gripsack — edit the module, not this block !!{suf}")
}

/// Locate a module's block by content, tolerant of indentation and
/// comment style (so a changed `marker` override between generations
/// still finds the old block). Returns (open line, close line).
fn find_block(existing: &str, module: &str) -> Option<(usize, usize)> {
    let open_key = format!(">>> gripsack module={module} >>>");
    let lines: Vec<&str> = existing.lines().collect();
    let open = lines.iter().position(|l| l.contains(&open_key))?;
    let close = lines[open + 1..]
        .iter()
        .position(|l| l.contains("<<< gripsack <<<"))
        .map(|i| open + 1 + i)?;
    Some((open, close))
}

/// The block's current content (between banner/markers), normalized
/// for hashing: lines joined, no trailing newline.
pub fn extract_block(existing: &str, module: &str) -> Option<String> {
    let lines: Vec<&str> = existing.lines().collect();
    let (open, close) = find_block(existing, module)?;
    Some(
        lines[open + 1..close]
            .iter()
            .filter(|l| !l.contains("!! managed by gripsack"))
            .copied()
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// Insert or replace the module's managed block. Regenerates the
/// whole block (conda's replace-not-merge: content drift inside the
/// markers self-heals on the next apply) and strips duplicate blocks
/// of the same module (conda's open TODO, closed here).
pub fn upsert_block(
    existing: &str,
    module: &str,
    dest: &Path,
    marker: Option<&str>,
    block: &str,
) -> Result<String, String> {
    let (open, close) = block_markers(module, dest, marker);
    let banner = banner(dest, marker);
    let crlf = existing.contains("\r\n");
    let generated: Vec<&str> = std::iter::once(open.as_str())
        .chain(std::iter::once(banner.as_str()))
        .chain(block.trim_end_matches('\n').lines())
        .chain(std::iter::once(close.as_str()))
        .collect();

    let mut lines: Vec<&str> = existing.lines().collect();
    if let Some((o, c)) = find_block(existing, module) {
        let mut out: Vec<&str> = Vec::with_capacity(lines.len() + generated.len());
        out.extend_from_slice(&lines[..o]);
        out.extend(generated.iter().copied());
        out.extend_from_slice(&lines[c + 1..]);
        // strip duplicate blocks of the same module that accumulated
        // behind the first (conda's TODO — duplicates never survive)
        let key = format!(">>> gripsack module={module} >>>");
        let mut cleaned: Vec<&str> = Vec::with_capacity(out.len());
        let mut kept_first = false;
        let mut skipping = false;
        for line in out {
            if !skipping && line.contains(&key) {
                if kept_first {
                    skipping = true;
                    continue;
                }
                kept_first = true;
            }
            if skipping {
                if line.contains("<<< gripsack <<<") {
                    skipping = false;
                }
                continue;
            }
            cleaned.push(line);
        }
        return Ok(join_lines(&cleaned, crlf));
    }
    if lines.last().is_some_and(|l| !l.trim().is_empty()) {
        lines.push("");
    }
    lines.extend(generated);
    Ok(join_lines(&lines, crlf))
}

/// Remove the module's block, returning the new content (None if the
/// block is absent). A trailing blank line left by an appended block
/// is trimmed; the caller deletes the file if what remains is empty.
pub fn remove_block(existing: &str, module: &str) -> Option<String> {
    let (open, close) = find_block(existing, module)?;
    let crlf = existing.contains("\r\n");
    let lines: Vec<&str> = existing.lines().collect();
    let mut out: Vec<&str> = Vec::with_capacity(lines.len());
    out.extend_from_slice(&lines[..open]);
    out.extend_from_slice(&lines[close + 1..]);
    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }
    if out.is_empty() {
        Some(String::new())
    } else {
        Some(join_lines(&out, crlf))
    }
}

fn join_lines(lines: &[&str], crlf: bool) -> String {
    let mut s = lines.join(if crlf { "\r\n" } else { "\n" });
    s.push_str(if crlf { "\r\n" } else { "\n" });
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn vars(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn template_substitutes_and_escapes() {
        let out = render_template(
            b"email = {{ email }}\nliteral = {{{{ not-a-var }}\n",
            &vars(&[("email", "a@b.c")]),
            "id",
        )
        .unwrap();
        assert_eq!(out, b"email = a@b.c\nliteral = {{ not-a-var }}\n");
    }

    #[test]
    fn template_undefined_variable_is_a_loud_error() {
        let err = render_template(b"{{ typo }}", &vars(&[("email", "a@b.c")]), "id").unwrap_err();
        let ExecError::Step { detail, .. } = &err else {
            panic!("expected ExecError::Step, got {err:?}");
        };
        assert!(detail.contains("undefined variable \"typo\""));
        assert!(detail.contains("email"));
    }

    #[test]
    fn template_unbalanced_braces_error() {
        assert!(render_template(b"x {{ y", &vars(&[]), "id").is_err());
    }

    #[test]
    fn merge_appends_block_to_foreign_file() {
        let dest = PathBuf::from("/home/u/.bashrc");
        let out = upsert_block(
            "# user stuff\nexport EDITOR=hx\n",
            "shell",
            &dest,
            None,
            "export PATH=\"$HOME/.local/bin:$PATH\"\n",
        )
        .unwrap();
        assert!(out.starts_with("# user stuff\nexport EDITOR=hx\n"));
        assert!(out.contains("# >>> gripsack module=shell >>>\n"));
        assert!(out.contains("# !! managed by gripsack"));
        assert!(out.contains("export PATH=\"$HOME/.local/bin:$PATH\"\n"));
        assert!(out.ends_with("# <<< gripsack <<<\n"));
    }

    #[test]
    fn merge_replaces_block_wholesale_and_self_heals_drift() {
        let dest = PathBuf::from("/home/u/.bashrc");
        let first = upsert_block("", "shell", &dest, None, "one\n").unwrap();
        let drifted = first.replace("one", "USER EDITED THIS");
        let second = upsert_block(&drifted, "shell", &dest, None, "two\n").unwrap();
        assert!(second.contains("two"));
        assert!(!second.contains("USER EDITED THIS"));
        assert!(!second.contains("one\n"));
    }

    #[test]
    fn merge_strips_duplicate_blocks() {
        let dest = PathBuf::from("/home/u/.bashrc");
        let first = upsert_block("", "shell", &dest, None, "one\n").unwrap();
        let doubled = format!("{first}{first}");
        let out = upsert_block(&doubled, "shell", &dest, None, "one\n").unwrap();
        assert_eq!(out.matches(">>> gripsack module=shell >>>").count(), 1);
    }

    #[test]
    fn merge_two_modules_coexist_in_one_file() {
        let dest = PathBuf::from("/home/u/.bashrc");
        let with_a = upsert_block("", "a", &dest, None, "A\n").unwrap();
        let with_both = upsert_block(&with_a, "b", &dest, None, "B\n").unwrap();
        assert!(with_both.contains("module=a"));
        assert!(with_both.contains("module=b"));
        let without_a = remove_block(&with_both, "a").unwrap();
        assert!(!without_a.contains("module=a"));
        assert!(without_a.contains("module=b"));
        assert!(without_a.contains("\nB\n"));
    }

    #[test]
    fn merge_comment_style_follows_the_destination() {
        assert_eq!(
            block_markers("m", &PathBuf::from("/u/.config/x.jsonc"), None).0,
            "// >>> gripsack module=m >>>"
        );
        assert_eq!(
            block_markers("m", &PathBuf::from("/u/.vimrc"), None).0,
            "\" >>> gripsack module=m >>>"
        );
        assert_eq!(
            block_markers("m", &PathBuf::from("/u/x.html"), None).1,
            "<!-- <<< gripsack <<< -->"
        );
        assert_eq!(
            block_markers("m", &PathBuf::from("/u/x.weird"), Some("#!")).0,
            "#! >>> gripsack module=m >>>"
        );
    }

    #[test]
    fn merge_preserves_crlf_files() {
        let dest = PathBuf::from("/u/.bashrc");
        let out = upsert_block("line one\r\n", "shell", &dest, None, "two\n").unwrap();
        assert!(out.contains("line one\r\n"));
        assert!(out.contains("# >>> gripsack module=shell >>>\r\n"));
    }

    #[test]
    fn extract_and_remove_roundtrip() {
        let dest = PathBuf::from("/u/.bashrc");
        let out = upsert_block("before\n", "shell", &dest, None, "the block\n").unwrap();
        assert_eq!(extract_block(&out, "shell").as_deref(), Some("the block"));
        assert_eq!(remove_block(&out, "shell").as_deref(), Some("before\n"));
        assert_eq!(remove_block("nothing here\n", "shell"), None);
    }
}
