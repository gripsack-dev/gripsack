//! E115 — path shape (0016 §D4). Validated symbolically at check time:
//! placeholders are opaque single-segment atoms (their values come from
//! a static table, and {version} values are git refnames — `..` is
//! impossible there by construction).
//!
//! - `from` (payload-relative): no absolute, no `~`, no `..`/`.`/empty
//!   segments, no trailing slash, no backslashes.
//! - `to`: no `..` segments (a `~/../` escape), never bare `~`/`~/`
//!   or `/` (the whole home or the filesystem root as a destination).

use crate::diagnostic::{Diagnostic, codes};
use crate::model::Ir;

/// Split on `/`, placeholders scrubbed to an opaque atom FIRST — they
/// appear inline (`bat-{version}-{target}/bat`), so splitting before
/// scrubbing manufactures phantom empty segments.
fn segments(path: &str) -> Vec<String> {
    let mut scrubbed = String::new();
    let mut rest = path;
    while let Some(open) = rest.find('{') {
        scrubbed.push_str(&rest[..open]);
        match rest[open..].find('}') {
            Some(close) => {
                scrubbed.push('P');
                rest = &rest[open + close + 1..];
            }
            None => {
                scrubbed.push_str(&rest[open..]);
                break;
            }
        }
    }
    scrubbed.push_str(rest);
    scrubbed.split('/').map(String::from).collect()
}

fn bad_segments(path: &str) -> Option<String> {
    let segs = segments(path);
    for (i, seg) in segs.iter().enumerate() {
        match seg.as_str() {
            ".." => return Some("`..` escapes are never allowed".into()),
            "." => return Some("`.` segments are meaningless".into()),
            "" if path != "/" && !(i == 0 && path.starts_with('/')) => {
                return Some("empty segment (a doubled or trailing slash)".into());
            }
            _ => {}
        }
        if seg.contains('\\') {
            return Some("backslashes are not path separators here".into());
        }
    }
    None
}

pub fn check(ir: &Ir, diagnostics: &mut Vec<Diagnostic>) {
    for (name, module) in &ir.modules {
        for entry in module.install.iter().chain(module.config.iter()) {
            let span = entry.span.clone().or_else(|| module.span.clone());
            // the empty `from` is the whole-payload form (an owned
            // symlink of the payload root) — nothing to validate
            if entry.from.is_empty() {
                continue;
            }
            // `from` is payload-relative
            if entry.from.starts_with('/') || entry.from.starts_with('~') {
                diagnostics.push(
                    Diagnostic::error(
                        codes::BAD_PATH,
                        format!(
                            "module {name:?}: source {:?} must be payload-relative",
                            entry.from
                        ),
                    )
                    .with_label(span.clone(), "entry declared here"),
                );
            } else if let Some(why) = bad_segments(&entry.from) {
                diagnostics.push(
                    Diagnostic::error(
                        codes::BAD_PATH,
                        format!("module {name:?}: source {:?} — {why}", entry.from),
                    )
                    .with_label(span.clone(), "entry declared here"),
                );
            }
            // `to`: escapes and whole-home/root destinations
            if entry.to == "~" || entry.to == "~/" || entry.to == "/" {
                diagnostics.push(
                    Diagnostic::error(
                        codes::BAD_PATH,
                        format!(
                            "module {name:?}: destination {:?} is far too broad",
                            entry.to
                        ),
                    )
                    .with_label(span.clone(), "entry declared here"),
                );
            } else if segments(&entry.to).iter().any(|s| s == "..") {
                diagnostics.push(
                    Diagnostic::error(
                        codes::BAD_PATH,
                        format!(
                            "module {name:?}: destination {:?} — `..` escapes are never allowed",
                            entry.to
                        ),
                    )
                    .with_label(span, "entry declared here"),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sema;

    fn ir_with(from: &str, to: &str) -> String {
        format!(
            r#"{{"ir_version": 1, "host": {{"os": "linux", "arch": "x86_64", "tags": [], "libc": "glibc-2.36"}}, "modules": {{"demo": {{
            "install": [{{"from": {from:?}, "to": {to:?}, "mode": "owned"}}],
            "span": {{"file": "modules/demo.ts", "line": 3, "col": 1}}}}}}}}"#
        )
    }

    fn path_errors(from: &str, to: &str) -> Vec<String> {
        let ir = crate::parse(&ir_with(from, to)).unwrap();
        sema::run(&ir)
            .iter()
            .filter(|d| d.code == codes::BAD_PATH)
            .map(|d| d.message.clone())
            .collect()
    }

    #[test]
    fn path_shape_rules() {
        assert!(path_errors("a/b.txt", "~/.config/demo/b.txt").is_empty());
        // placeholders are opaque single segments
        assert!(path_errors("bat-{version}-{target}/bat", "~/.local/bin/bat").is_empty());
        assert!(!path_errors("../escape.txt", "~/x").is_empty());
        assert!(!path_errors("/abs.txt", "~/x").is_empty());
        assert!(!path_errors("a//b.txt", "~/x").is_empty());
        assert!(!path_errors("a/./b.txt", "~/x").is_empty());
        assert!(!path_errors("a/b.txt", "~/../escape").is_empty());
        assert!(!path_errors("a/b.txt", "~/").is_empty());
        // the whole-payload form is legal (tpm's directory symlink)
        assert!(path_errors("", "~/.config/tmux/plugins/tpm").is_empty());
    }
}
