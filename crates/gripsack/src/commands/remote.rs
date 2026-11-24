//! Trust on first use for remote repos (#409). A repo fetched from a git URL is
//! confirmed once per origin URL, showing what it declares; later commits from
//! the same origin are not re-prompted. Local repos need no record at all.
use super::guard::{self, Findings};
use std::{
    collections::BTreeSet,
    io::{self, Read},
    path::{Path, PathBuf},
    process::ExitCode,
};

const KNOWN: &str = "known-remotes";
const MAX_BYTES: u64 = 1024 * 1024;

/// Credential-free, case-folded identity of an origin URL. The same origin
/// spelled with a trailing slash, `.git` or embedded credentials is one key.
pub fn normalize(url: &str) -> String {
    let url = url.trim();
    let (scheme, rest) = match url.split_once("://") {
        Some((scheme, rest)) => (Some(scheme.to_ascii_lowercase()), rest),
        None => (None, url),
    };
    let rest = if scheme.is_some() {
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        format!("{}/{path}", host.to_ascii_lowercase())
    } else {
        rest.to_owned()
    };
    let rest = rest.trim_end_matches('/');
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    match scheme {
        Some(scheme) => format!("{scheme}://{rest}"),
        None => rest.to_owned(),
    }
}

fn load(home: &Path) -> BTreeSet<String> {
    let read = || -> io::Result<String> {
        let directory = gripsack_host::fs::open(home)?;
        let file = gripsack_host::fs::open_file_nofollow(&directory, Path::new(KNOWN))?;
        let mut text = String::new();
        file.take(MAX_BYTES).read_to_string(&mut text)?;
        Ok(text)
    };
    // A missing or unreadable list only means the next remote is asked about.
    read()
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

fn save(home: &Path, known: &BTreeSet<String>) -> io::Result<()> {
    let directory = gripsack_host::fs::open_or_create(home)?;
    let mut text = String::new();
    for entry in known {
        text.push_str(entry);
        text.push('\n');
    }
    gripsack_host::fs::atomic_write_with_mode(&directory, Path::new(KNOWN), text.as_bytes(), 0o600)
}

pub fn is_trusted(home: &Path, origin: &str) -> bool {
    load(home).contains(&normalize(origin))
}

pub fn trust(home: &Path, origin: &str) -> io::Result<()> {
    let mut known = load(home);
    known.insert(normalize(origin));
    save(home, &known)
}

pub fn forget(home: &Path, origin: &str) -> io::Result<bool> {
    let mut known = load(home);
    let removed = known.remove(&normalize(origin));
    if removed {
        save(home, &known)?;
    }
    Ok(removed)
}

pub fn list(home: &Path) -> Vec<String> {
    load(home).into_iter().collect()
}

/// The origin URL when `repo` is a cache clone made for `--repo <git url>`.
pub fn origin_of(home: &Path, repo: &Path) -> Option<String> {
    let cache = home.join("repos").canonicalize().ok()?;
    let repo: PathBuf = repo.canonicalize().ok()?;
    if !repo.starts_with(&cache) {
        return None;
    }
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["config", "--get", "remote.origin.url"])
        .output()
        .ok()
        .filter(|output| output.status.success())?;
    let url = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (!url.is_empty()).then_some(url)
}

/// First use of a remote origin: show what the repo declares and ask once.
/// `--yes` answers for scripts; without a terminal and without `--yes` the
/// remote is refused. Returns Err after reporting when the remote is not
/// trusted.
pub fn confirm_first_use(home: &Path, origin: &str, found: &Findings) -> Result<(), ExitCode> {
    if is_trusted(home, origin) {
        return Ok(());
    }
    let shown = gripsack_host::process::terminal::tame(normalize(origin));
    println!("first use of remote repo {shown}");
    if found.is_empty() {
        println!("it declares no hooks, fetch domains or destinations outside ~");
    } else {
        print!("{}", found.render("it declares:"));
    }
    println!(
        "later commits from this origin will not ask again; `grip trust remove <url>` revokes"
    );
    let accepted = if guard::assume_yes() {
        true
    } else if guard::interactive() {
        guard::confirm("trust this remote?")
    } else {
        eprintln!(
            "grip: remote repo {shown} is not trusted yet; re-run with --yes to trust it on first use"
        );
        return Err(ExitCode::FAILURE);
    };
    if !accepted {
        eprintln!("grip: remote repo not trusted — nothing was applied");
        return Err(ExitCode::FAILURE);
    }
    trust(home, origin).map_err(|error| {
        eprintln!("grip: cannot record the trusted remote: {error}");
        ExitCode::FAILURE
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spellings_of_one_origin_share_a_key_without_credentials() {
        let key = normalize("https://User:tok@GitHub.com/Me/env.git/");
        assert_eq!(key, "https://github.com/Me/env");
        assert_eq!(normalize("https://github.com/Me/env"), key);
        assert_eq!(
            normalize("git@github.com:me/env.git"),
            "git@github.com:me/env"
        );
        assert_ne!(normalize("https://github.com/Me/other"), key);
    }

    #[test]
    fn trust_is_recorded_per_origin_and_revocable() {
        let home = tempfile::tempdir().unwrap();
        assert!(!is_trusted(home.path(), "https://example.test/a.git"));
        trust(home.path(), "https://example.test/a.git").unwrap();
        assert!(is_trusted(home.path(), "https://example.test/a"));
        assert!(!is_trusted(home.path(), "https://example.test/b"));
        assert_eq!(list(home.path()), ["https://example.test/a"]);
        assert!(forget(home.path(), "https://example.test/a.git").unwrap());
        assert!(!is_trusted(home.path(), "https://example.test/a"));
        assert!(!forget(home.path(), "https://example.test/a").unwrap());
    }
}
