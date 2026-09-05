//! `git:` — pinned to an immutable rev; a shallow fetch of the exact
//! sha keeps it fast (works on GitHub and any server with
//! allowReachableSHA1InWant).

use super::FetchError;
use std::path::Path;

/// The remote's default-branch HEAD — the float resolution for a
/// rev-less git spec (0016 §D2). Runs at lock/update time; the sha it
/// returns is what every apply fetches until `grip update`.
pub fn resolve_head(url: &str) -> Result<String, FetchError> {
    let out = std::process::Command::new("git")
        .args(["ls-remote", url, "HEAD"])
        .output()?;
    if !out.status.success() {
        return Err(FetchError::Http {
            url: url.to_string(),
            reason: format!("git ls-remote exited {}", out.status),
        });
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let sha = text.split_whitespace().next().unwrap_or("");
    if sha.len() != 40 && sha.len() != 64 {
        return Err(FetchError::Http {
            url: url.to_string(),
            reason: format!("git ls-remote returned no HEAD sha (got {sha:?})"),
        });
    }
    Ok(sha.to_string())
}

pub(crate) fn fetch(url: &str, rev: &str, dest: &Path) -> Result<String, FetchError> {
    // rev flows into `git fetch origin <rev>` as an argument: a value
    // like `--upload-pack=<cmd>` is option injection, not a rev.
    // Shas and plain ref names are all hex/dots/slashes/alnum.
    if !rev
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/'))
        || rev.is_empty()
    {
        return Err(FetchError::Http {
            url: url.to_string(),
            reason: format!("invalid rev {rev:?}: expected a sha or ref name"),
        });
    }
    let git = |args: &[&str]| {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(dest)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(FetchError::Http {
                url: url.to_string(),
                reason: format!("git {:?} exited {status}", args),
            })
        }
    };
    git(&["init", "--quiet"])?;
    git(&["remote", "add", "origin", url])?;
    git(&["fetch", "--quiet", "--depth", "1", "origin", rev])?;
    git(&["checkout", "--quiet", "FETCH_HEAD"])?;
    // the checkout is the payload; .git is fetch machinery — pack
    // layout differs per fetch (non-deterministic hash, every apply
    // re-pinned) and it must never reach the store
    std::fs::remove_dir_all(dest.join(".git"))?;
    // the rev is the pin; the tree hash is the payload identity
    Ok(gripsack_store::canonical_tree_hash(dest)?.to_string())
}

#[cfg(test)]
mod tests {
    use super::super::FetchSpec;
    use super::super::fetch;
    use super::*;
    use std::fs;

    #[test]
    fn git_fetch_clones_at_rev() {
        let dir = tempfile::tempdir().unwrap();
        let remote = dir.path().join("remote");
        fs::create_dir_all(&remote).unwrap();
        let git = |args: &[&str], cwd: &Path| {
            assert!(
                std::process::Command::new("git")
                    .args(args)
                    .current_dir(cwd)
                    .env("GIT_AUTHOR_NAME", "t")
                    .env("GIT_AUTHOR_EMAIL", "t@t")
                    .env("GIT_COMMITTER_NAME", "t")
                    .env("GIT_COMMITTER_EMAIL", "t@t")
                    .status()
                    .unwrap()
                    .success()
            );
        };
        git(&["init", "--quiet"], &remote);
        fs::write(remote.join("file.txt"), b"v1\n").unwrap();
        git(&["add", "."], &remote);
        git(&["commit", "--quiet", "-m", "init"], &remote);
        let out = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&remote)
            .output()
            .unwrap();
        let rev = String::from_utf8(out.stdout).unwrap().trim().to_string();

        let dest = dir.path().join("out");
        let hash = fetch(
            &FetchSpec::Git {
                url: remote.to_string_lossy().into_owned(),
                rev: Some(rev.clone()),
            },
            &dest,
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(dest.join("file.txt")).unwrap(),
            "v1\n"
        );
        assert!(!hash.is_empty());

        // the checkout is the payload: no .git reaches the store, and
        // the same rev hashes identically on every fetch (a mismatch
        // per fetch made every apply fail its pin check)
        assert!(!dest.join(".git").exists());
        let dest2 = dir.path().join("out2");
        let hash2 = fetch(
            &FetchSpec::Git {
                url: remote.to_string_lossy().into_owned(),
                rev: Some(rev),
            },
            &dest2,
        )
        .unwrap();
        assert_eq!(hash, hash2, "same rev must hash identically");
    }
}
