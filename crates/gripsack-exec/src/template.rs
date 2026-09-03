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

/// The generated marker lines for a module's block, as text:
///
/// `{pre} >>> gripsack module={m} sha={16hex} >>>{suf}` / banner /
/// content / `{pre} <<< gripsack module={m} <<<{suf}`.
///
/// Both markers carry the module name (a payload line quoting marker
/// text is never mistaken for one — the 0.17.13 corruption class),
/// and the open marker carries the block's content hash: the block is
/// self-describing, so "edited since deploy" is detectable from the
/// file alone, without the generation manifest.
fn open_marker(module: &str, sha: &str, pre: &str, suf: &str) -> String {
    format!("{pre} >>> gripsack module={module} sha={sha} >>>{suf}")
}

fn close_marker(module: &str, pre: &str, suf: &str) -> String {
    format!("{pre} <<< gripsack module={module} <<<{suf}")
}

/// The `sha=<hex>` recorded in a module's open marker — the content
/// hash at deploy time. A mismatch with the block's current hash
/// means someone edited inside the markers since.
pub fn marker_sha(existing: &str, module: &str) -> Option<String> {
    let (open, _) = find_block(existing, module)?;
    let line = existing.lines().nth(open)?;
    let after = line.split("sha=").nth(1)?;
    let sha = after.split_whitespace().next()?;
    Some(sha.trim_end_matches(['>', '-']).to_string())
}

/// Is this line THE close-marker line for `module`? Tolerant of
/// comment style (any single-token prefix, html `-->` tail) but never
/// of content around the key.
fn line_is_close_marker(line: &str, module: &str) -> bool {
    let key = format!("<<< gripsack module={module} <<<");
    line_has_shape(line, &key, false)
}

/// Is this line the open marker for `module`? The key is the module
/// prefix; the tail must be `sha=<hex> >>>`.
fn line_is_open_marker(line: &str, module: &str) -> bool {
    let prefix = format!(">>> gripsack module={module}");
    line_has_shape(line, &prefix, true)
}

/// Marker-line shape check: `key` appears verbatim, nothing but a
/// comment prefix before it, and the tail after it is empty or `-->`
/// (close markers), or `sha=<hex> >>>` (open markers).
fn line_has_shape(line: &str, key: &str, open_marker: bool) -> bool {
    let trimmed = line.trim();
    let Some(idx) = trimmed.find(key) else {
        return false;
    };
    let before = trimmed[..idx].trim();
    let tail = trimmed[idx + key.len()..].trim();
    let prefix_ok = before.is_empty() || before.split_whitespace().count() == 1;
    let tail_ok = if !open_marker {
        tail.is_empty() || tail == "-->"
    } else {
        // `sha=<hex> >>>` — the hash, the comment closer, nothing else
        let Some(rest) = tail.strip_prefix("sha=") else {
            return false;
        };
        let mut tokens = rest.split_whitespace();
        let hex = tokens.next().unwrap_or_default();
        let end = tokens.next().unwrap_or_default();
        !hex.is_empty() && (end == ">>>" || end == "-->") && tokens.next().is_none()
    };
    prefix_ok && tail_ok
}

/// Locate a module's block: (open line, close line), zero-based.
fn find_block(existing: &str, module: &str) -> Option<(usize, usize)> {
    let lines: Vec<&str> = existing.lines().collect();
    let open = lines.iter().position(|l| line_is_open_marker(l, module))?;
    let close = lines[open + 1..]
        .iter()
        .position(|l| line_is_close_marker(l, module))
        .map(|i| open + 1 + i)?;
    Some((open, close))
}

/// The block's current content (between the banner and the close
/// marker), normalized for hashing: lines joined, no trailing newline.
pub fn extract_block(existing: &str, module: &str) -> Option<String> {
    let lines: Vec<&str> = existing.lines().collect();
    let (open, close) = find_block(existing, module)?;
    // the banner is generated as the FIRST line after the open
    // marker — skip exactly that one
    let start = if lines
        .get(open + 1)
        .is_some_and(|l| l.contains("!! managed by gripsack"))
    {
        open + 2
    } else {
        open + 1
    };
    Some(lines[start..close].join("\n"))
}

/// Insert or replace the module's managed block. Regenerates the
/// whole block (conda's replace-not-merge: content drift inside the
/// markers self-heals on the next apply) and strips duplicate blocks
/// of the same module that accumulated behind the first.
pub fn upsert_block(
    existing: &str,
    module: &str,
    dest: &Path,
    marker: Option<&str>,
    block: &str,
) -> Result<String, String> {
    let (pre, suf) = comment_style(dest, marker);
    let block = block.trim_end_matches('\n');
    let sha = &gripsack_store::canonical_bytes_hash(block.as_bytes())[..16];
    let open = open_marker(module, sha, &pre, suf);
    let close = close_marker(module, &pre, suf);
    let banner = format!("{pre} !! managed by gripsack — edit the module, not this block !!{suf}");
    let crlf = existing.contains("\r\n");
    let generated: Vec<&str> = std::iter::once(open.as_str())
        .chain(std::iter::once(banner.as_str()))
        .chain(block.lines())
        .chain(std::iter::once(close.as_str()))
        .collect();

    let mut lines: Vec<&str> = existing.lines().collect();
    if let Some((o, c)) = find_block(existing, module) {
        let mut out: Vec<&str> = Vec::with_capacity(lines.len() + generated.len());
        out.extend_from_slice(&lines[..o]);
        out.extend(generated.iter().copied());
        out.extend_from_slice(&lines[c + 1..]);
        // strip duplicate blocks of the same module that accumulated
        // behind the first — duplicates never survive
        let mut cleaned: Vec<&str> = Vec::with_capacity(out.len());
        let mut kept_first = false;
        let mut skipping = false;
        for line in out {
            if !skipping && line_is_open_marker(line, module) {
                if kept_first {
                    skipping = true;
                    continue;
                }
                kept_first = true;
            }
            if skipping {
                if line_is_close_marker(line, module) {
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
        assert!(out.contains(">>> gripsack module=shell sha="));
        assert!(out.contains("# !! managed by gripsack"));
        assert!(out.contains("export PATH=\"$HOME/.local/bin:$PATH\"\n"));
        assert!(out.ends_with("# <<< gripsack module=shell <<<\n"));
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
        assert_eq!(out.matches(">>> gripsack module=shell sha=").count(), 1);
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
        // upsert produces the markers; assert their comment style and
        // grammar (open carries sha, close does not)
        let out = upsert_block("", "m", &PathBuf::from("/u/.config/x.jsonc"), None, "x\n").unwrap();
        assert!(out.starts_with("// >>> gripsack module=m sha="), "{out}");
        assert!(
            out.ends_with("<<< gripsack module=m <<<\n")
                || out.contains("<<< gripsack module=m <<< -->")
        );

        let out = upsert_block("", "m", &PathBuf::from("/u/.vimrc"), None, "x\n").unwrap();
        assert!(out.starts_with("\" >>> gripsack module=m sha="));

        let out = upsert_block("", "m", &PathBuf::from("/u/x.html"), None, "x\n").unwrap();
        assert!(out.starts_with("<!-- >>> gripsack module=m sha="));
        assert!(out.contains("<<< gripsack module=m <<< -->"));

        let out = upsert_block("", "m", &PathBuf::from("/u/x.weird"), Some("#!"), "x\n").unwrap();
        assert!(out.starts_with("#! >>> gripsack module=m sha="));
    }

    #[test]
    fn merge_preserves_crlf_files() {
        let dest = PathBuf::from("/u/.bashrc");
        let out = upsert_block("line one\r\n", "shell", &dest, None, "two\n").unwrap();
        assert!(out.contains("line one\r\n"));
        assert!(out.contains(">>> gripsack module=shell sha="));
    }

    #[test]
    fn payload_quoting_marker_text_does_not_break_the_block() {
        let dest = PathBuf::from("/home/u/.bashrc");
        // a payload line that literally quotes the old close marker —
        // block detection must not read it as the end of the block
        let payload = "echo 'docs say <<< gripsack <<< ends a block'\n";
        let out = upsert_block("# rc\n", "m", &dest, None, payload).unwrap();
        assert_eq!(out.matches(">>> gripsack module=m sha=").count(), 1);
        // re-apply is idempotent, not appending strays forever
        let again = upsert_block(&out, "m", &dest, None, payload).unwrap();
        assert_eq!(again, out, "second apply must not grow the file");
        // the whole payload is inside the block
        assert!(
            extract_block(&out, "m")
                .unwrap()
                .contains("<<< gripsack <<<")
        );
        // and removing the block leaves exactly the user's file
        assert_eq!(remove_block(&out, "m").unwrap(), "# rc\n");
    }

    #[test]
    fn marker_sha_detects_hand_edits() {
        let dest = PathBuf::from("/home/u/.bashrc");
        let out = upsert_block("", "m", &dest, None, "original\n").unwrap();
        let sha = marker_sha(&out, "m").expect("marker carries the sha");
        assert_eq!(sha.len(), 16);
        assert_eq!(
            sha,
            &gripsack_store::canonical_bytes_hash(b"original")[..16]
        );

        // a hand edit inside the markers: the content moves, the
        // recorded sha does not — the mismatch is detectable from the
        // file alone
        let edited = out.replace("original", "hand-edited");
        assert_eq!(marker_sha(&edited, "m").as_deref(), Some(sha.as_str()));
        assert_ne!(
            &gripsack_store::canonical_bytes_hash(extract_block(&edited, "m").unwrap().as_bytes())
                [..16],
            sha.as_str()
        );
    }

    #[test]
    fn banner_text_inside_payload_is_not_dropped_from_the_hash() {
        let dest = PathBuf::from("/home/u/.bashrc");
        let payload = "echo '!! managed by gripsack says the docs'\n";
        let out = upsert_block("", "m", &dest, None, payload).unwrap();
        let extracted = extract_block(&out, "m").unwrap();
        assert!(
            extracted.contains("!! managed by gripsack"),
            "payload quoting the banner must stay in the block hash"
        );
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
