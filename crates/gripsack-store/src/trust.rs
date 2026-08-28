//! Repo trust (plan/0013 D7): the gate before any frontend eval.
//!
//! An env repo is code. Before grip evaluates one, the repo must be
//! explicitly trusted: `$GRIPSACK_HOME/trust.toml` holds one
//! `[[repos]]` entry per repo, keyed by canonical path — a moved or
//! re-cloned repo re-prompts (the `git safe.directory` precedent),
//! a new commit does not (per-commit keys would train users to bypass
//! the gate on their own dotfiles).
//!
//! ```toml
//! [[repos]]
//! path = "/home/tarek/myenv"                 # canonical; the trust key
//! remote = "git@github.com:tarek/myenv"      # informational
//! commit = "54d91a1…"                        # recorded for audit
//! trusted_at = "2026-08-28T12:00:00Z"
//! ```
//!
//! Remote and commit come from the `git` CLI — the same dependency
//! the `--repo` clone path already has; there is no libgit2 in the
//! tree and trust does not need one.

use serde::{Deserialize, Serialize};
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use crate::fs::atomic_write;
use crate::paths::gripsack_home;

/// The exact capability set eval gets (0013 D2/D7) — shown at the
/// prompt so the trust decision is informed. Wording is contract.
const SANDBOX_SUMMARY: &str = "eval runs sandboxed TypeScript — no environment variables, no network, no subprocesses, read-only within the repo";

/// One `[[repos]]` entry in trust.toml.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustedRepo {
    /// Canonical repo path — the trust key.
    pub path: String,
    /// `origin` remote when the repo is a git checkout. Informational.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
    /// HEAD at trust time. Audit trail, not a key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// RFC3339 UTC timestamp of the trust decision.
    pub trusted_at: String,
}

/// The whole trust file. `repos` is the only table.
#[derive(Debug, Default, Serialize, Deserialize)]
struct TrustFile {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    repos: Vec<TrustedRepo>,
}

/// The trust gate (0013 D7). Ok = the repo may be evaluated; Err = it
/// may not (print and fail). Every command that evals calls this
/// before the first eval; it is idempotent — once a TTY prompt
/// records the repo, later gates are a file lookup.
///
/// `GRIPSACK_TRUST_ALL=1` is the documented CI escape hatch.
pub fn ensure_trusted(repo: &Path) -> io::Result<()> {
    if std::env::var_os("GRIPSACK_TRUST_ALL").is_some_and(|v| v == *"1") {
        return Ok(());
    }
    ensure_trusted_at(&gripsack_home(), repo, stdin_and_stdout_are_tty())
}

/// Testable core of [`ensure_trusted`]: gate `repo` against the
/// trust list in `home`, prompting only when `interactive`.
fn ensure_trusted_at(home: &Path, repo: &Path, interactive: bool) -> io::Result<()> {
    let key = canonical_key(repo);
    let mut file = load(home)?;
    if file.repos.iter().any(|r| Path::new(&r.path) == key) {
        return Ok(());
    }
    let remote = remote_of(repo);
    let commit = head_of(repo);
    if !interactive {
        return Err(io::Error::other(format!(
            "untrusted repo {} — run `grip trust add {}` to trust it, or set GRIPSACK_TRUST_ALL=1",
            key.display(),
            key.display()
        )));
    }
    if !prompt_tty(&key, &remote, &commit)? {
        return Err(io::Error::other(format!(
            "trust declined for {} — no eval was run",
            key.display()
        )));
    }
    let entry = entry_for(&key, remote, commit);
    record(&mut file, entry);
    save(home, &file)
}

/// Recorded entries, in file order.
pub fn list(home: &Path) -> io::Result<Vec<TrustedRepo>> {
    Ok(load(home)?.repos)
}

/// Record `repo` as trusted. Upsert: re-adding refreshes remote,
/// commit, and timestamp instead of duplicating. Returns the entry
/// written — the canonical path is in `entry.path`.
pub fn add(home: &Path, repo: &Path) -> io::Result<TrustedRepo> {
    let mut file = load(home)?;
    let entry = entry_for(&canonical_key(repo), remote_of(repo), head_of(repo));
    record(&mut file, entry.clone());
    save(home, &file)?;
    Ok(entry)
}

/// Forget `repo` (by canonical path). `true` when an entry was
/// removed; `false` when it was never trusted.
pub fn remove(home: &Path, repo: &Path) -> io::Result<bool> {
    let mut file = load(home)?;
    let key = canonical_key(repo);
    let before = file.repos.len();
    file.repos.retain(|r| Path::new(&r.path) != key);
    let removed = file.repos.len() != before;
    if removed {
        save(home, &file)?;
    }
    Ok(removed)
}

/// Is this repo in the trust list, by canonical path? A missing or
/// unreadable trust file is simply "no" — the gate re-asks and
/// surfaces the real error if there is one.
pub fn is_trusted(home: &Path, repo: &Path) -> bool {
    let key = canonical_key(repo);
    load(home)
        .map(|f| f.repos.iter().any(|r| Path::new(&r.path) == key))
        .unwrap_or(false)
}

/// The trust key: the canonical path. A nonexistent path falls back
/// to absolute, so `trust add` of a not-yet-cloned repo still
/// records a stable key that the canonical form will match later.
pub fn canonical_key(path: &Path) -> PathBuf {
    std::fs::canonicalize(path)
        .or_else(|_| std::path::absolute(path))
        .unwrap_or_else(|_| path.to_path_buf())
}

/// `$GRIPSACK_HOME/trust.toml`.
pub fn trust_file(home: &Path) -> PathBuf {
    home.join("trust.toml")
}

/// Upsert one entry, keyed by canonical path.
fn record(file: &mut TrustFile, entry: TrustedRepo) {
    match file.repos.iter().position(|r| r.path == entry.path) {
        Some(i) => file.repos[i] = entry,
        None => file.repos.push(entry),
    }
}

fn entry_for(key: &Path, remote: Option<String>, commit: Option<String>) -> TrustedRepo {
    TrustedRepo {
        path: key.display().to_string(),
        remote,
        commit,
        trusted_at: now_rfc3339(),
    }
}

/// Load the trust file; missing file = empty trust list, not an
/// error (first run has never trusted anything).
fn load(home: &Path) -> io::Result<TrustFile> {
    let path = trust_file(home);
    match std::fs::read_to_string(&path) {
        Ok(text) => toml::from_str(&text)
            .map_err(|e| io::Error::other(format!("cannot parse {}: {e}", path.display()))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(TrustFile::default()),
        Err(e) => Err(e),
    }
}

fn save(home: &Path, file: &TrustFile) -> io::Result<()> {
    let text = toml::to_string(file)
        .map_err(|e| io::Error::other(format!("cannot serialize trust list: {e}")))?;
    atomic_write(&trust_file(home), text.as_bytes())
}

/// Ask on the TTY; `true` only on an explicit `y` (default N, EOF
/// and anything else decline).
fn prompt_tty(key: &Path, remote: &Option<String>, commit: &Option<String>) -> io::Result<bool> {
    let mut out = io::stdout().lock();
    let _ = writeln!(out, "first eval of this repo — trust it?");
    let _ = writeln!(out);
    let _ = writeln!(out, "  path:    {}", key.display());
    let _ = writeln!(out, "  remote:  {}", remote.as_deref().unwrap_or("(none)"));
    let _ = writeln!(
        out,
        "  commit:  {}",
        commit.as_deref().map(short_sha).unwrap_or("(none)")
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "{}", wrap_indent(SANDBOX_SUMMARY, 62));
    let _ = writeln!(out);
    let _ = write!(out, "trust this repo? [y/N] ");
    out.flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    Ok(answer.trim().eq_ignore_ascii_case("y"))
}

fn stdin_and_stdout_are_tty() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

/// 7-char commit prefix, git-short style.
fn short_sha(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}

/// `git -C <repo> <args…>` → trimmed stdout, or None (not a repo, no
/// git, or the query failed — all mean "nothing to record").
fn git(repo: &Path, args: &[&str]) -> Option<String> {
    std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

fn remote_of(repo: &Path) -> Option<String> {
    git(repo, &["remote", "get-url", "origin"])
}

fn head_of(repo: &Path) -> Option<String> {
    git(repo, &["rev-parse", "HEAD"])
}

/// RFC3339 UTC for "now". Hand-rolled (civil-from-days below): a
/// write-once audit stamp does not justify a datetime dependency.
fn now_rfc3339() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    rfc3339(secs)
}

fn rfc3339(epoch_secs: u64) -> String {
    let (y, m, d) = civil_from_days((epoch_secs / 86_400) as i64);
    let s = epoch_secs % 86_400;
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        s / 3600,
        (s % 3600) / 60,
        s % 60
    )
}

/// Days since 1970-01-01 → (year, month, day), proleptic Gregorian —
/// Hinnant's `civil_from_days`.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = yoe + era * 400 + u64::from(m <= 2) as i64;
    (y, m as u32, d as u32)
}

/// Two-space-indented word wrap for the prompt block.
fn wrap_indent(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut line = String::from("  ");
    for word in text.split_whitespace() {
        if line.len() > 2 && line.len() + 1 + word.len() > width + 2 {
            out.push_str(line.trim_end());
            out.push('\n');
            line = String::from("  ");
        }
        if line.len() > 2 {
            line.push(' ');
        }
        line.push_str(word);
    }
    out.push_str(line.trim_end());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_home() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    fn dir_under(home: &Path, name: &str) -> PathBuf {
        let p = home.join(name);
        std::fs::create_dir_all(&p).expect("mkdir");
        p
    }

    #[test]
    fn trust_toml_round_trip() {
        let home = new_home();
        // no file yet → empty list, not an error
        assert!(list(home.path()).unwrap().is_empty());

        let repo = dir_under(home.path(), "myenv");
        let added = add(home.path(), &repo).unwrap();
        // tempdir is not a git repo → nothing to record but the path
        assert_eq!(added.remote, None);
        assert_eq!(added.commit, None);
        assert_eq!(added.path, repo.display().to_string());
        assert_eq!(list(home.path()).unwrap(), vec![added.clone()]);

        // the on-disk shape is the contract's (0013 D7)
        let text = std::fs::read_to_string(trust_file(home.path())).unwrap();
        assert!(text.starts_with("[[repos]]"), "{text}");
        assert!(
            text.contains(&format!("path = \"{}\"", repo.display())),
            "{text}"
        );
        assert!(text.contains("trusted_at = \""), "{text}");
        assert!(!text.contains("remote"), "{text}"); // None is skipped

        // re-add refreshes in place — never a duplicate
        std::fs::write(repo.join("marker"), b"x").unwrap();
        add(home.path(), &repo).unwrap();
        let loaded = list(home.path()).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].path, added.path);
    }

    #[test]
    fn trust_is_keyed_on_canonical_path() {
        let home = new_home();
        let repo = dir_under(home.path(), "real");
        let alias = home.path().join("alias");
        std::os::unix::fs::symlink(&repo, &alias).unwrap();

        add(home.path(), &alias).unwrap(); // recorded via the alias…
        assert!(is_trusted(home.path(), &repo)); // …keyed on the target
        assert!(is_trusted(home.path(), &alias));

        // gate passes for the canonical form
        ensure_trusted_at(home.path(), &repo, false).unwrap();

        // a sibling is not trusted
        let other = dir_under(home.path(), "other");
        assert!(!is_trusted(home.path(), &other));

        // moving the repo re-prompts — that is the case that matters
        let moved = home.path().join("moved");
        std::fs::rename(&repo, &moved).unwrap();
        assert!(!is_trusted(home.path(), &moved));
    }

    #[test]
    fn gate_blocks_untrusted_repos_without_a_tty() {
        let home = new_home();
        let repo = dir_under(home.path(), "myenv");

        let err = ensure_trusted_at(home.path(), &repo, false).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("untrusted repo"), "{msg}");
        assert!(msg.contains(&repo.display().to_string()), "{msg}");
        assert!(msg.contains("`grip trust add"), "{msg}");
        assert!(msg.contains("GRIPSACK_TRUST_ALL=1"), "{msg}");
    }

    #[test]
    fn remove_forgets_the_repo() {
        let home = new_home();
        let repo = dir_under(home.path(), "myenv");
        add(home.path(), &repo).unwrap();
        assert!(is_trusted(home.path(), &repo));

        assert!(remove(home.path(), &repo).unwrap());
        assert!(!is_trusted(home.path(), &repo));
        ensure_trusted_at(home.path(), &repo, false).unwrap_err(); // gate re-arms

        assert!(!remove(home.path(), &repo).unwrap()); // nothing left
    }

    #[test]
    fn canonical_key_falls_back_to_absolute() {
        assert_eq!(
            canonical_key(Path::new("/nonexistent/repo")),
            Path::new("/nonexistent/repo")
        );
        assert!(canonical_key(Path::new("relative/repo")).is_absolute());
    }

    #[test]
    fn only_an_explicit_y_trusts() {
        assert!(prompt_answer_is_yes("y\n"));
        assert!(prompt_answer_is_yes(" Y \n"));
        assert!(!prompt_answer_is_yes("\n")); // default N
        assert!(!prompt_answer_is_yes("n\n"));
        assert!(!prompt_answer_is_yes("yes\n")); // the prompt says [y/N]
        assert!(!prompt_answer_is_yes("")); // EOF
    }

    fn prompt_answer_is_yes(answer: &str) -> bool {
        answer.trim().eq_ignore_ascii_case("y")
    }

    #[test]
    fn rfc3339_known_instants() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(86_399), "1970-01-01T23:59:59Z");
        assert_eq!(rfc3339(1_000_000_000), "2001-09-09T01:46:40Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z"); // leap day
        assert_eq!(rfc3339(1_709_164_800), "2024-02-29T00:00:00Z"); // leap day
    }

    #[test]
    fn sandbox_summary_survives_wrapping_intact() {
        let wrapped = wrap_indent(SANDBOX_SUMMARY, 62);
        assert_eq!(
            wrapped
                .replace('\n', " ")
                .split_whitespace()
                .collect::<Vec<_>>(),
            SANDBOX_SUMMARY.split_whitespace().collect::<Vec<_>>()
        );
        for line in wrapped.lines() {
            assert!(line.starts_with("  "));
        }
    }
}
