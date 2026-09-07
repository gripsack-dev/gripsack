//! `git:` — pinned to an immutable rev; a shallow fetch of the exact
//! sha keeps it fast (works on GitHub and any server with
//! allowReachableSHA1InWant).

use super::FetchError;
use std::path::Path;

/// The remote's default-branch HEAD — the float resolution for a
/// rev-less git spec (0016 §D2). Runs at lock/update time; the sha it
/// returns is what every apply fetches until `grip update`.
pub fn resolve_head(url: &str) -> Result<String, FetchError> {
    let mut command = std::process::Command::new("git");
    command.args(["ls-remote", "--", url, "HEAD"]);
    let out = run(&mut command, url)?;
    let text = String::from_utf8_lossy(&out);
    let sha = text.split_whitespace().next().unwrap_or("");
    if !matches!(sha.len(), 40 | 64) || !sha.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(FetchError::Http {
            url: url.into(),
            reason: "git ls-remote returned no HEAD object ID".into(),
        });
    }
    Ok(sha.to_owned())
}

fn run(command: &mut std::process::Command, url: &str) -> Result<Vec<u8>, FetchError> {
    let mut stdout = Vec::new();
    let limits = gripsack_process::Limits {
        stdout_bytes: 16 * 1024,
        line_bytes: 16 * 1024,
        ..Default::default()
    };
    let outcome = gripsack_process::run(command, &[], limits, |line| {
        stdout.extend_from_slice(line);
        stdout.push(b'\n');
        gripsack_process::Control::Continue
    })?;
    if !matches!(outcome.reason, gripsack_process::StopReason::Exited)
        || !outcome.status.is_some_and(|status| status.success())
    {
        return Err(FetchError::Http {
            url: url.into(),
            reason: format!(
                "git stopped ({:?}, {:?}): {}",
                outcome.reason,
                outcome.status,
                String::from_utf8_lossy(&outcome.stderr)
            ),
        });
    }
    Ok(stdout)
}

pub(crate) fn fetch(
    context: &crate::FetchContext,
    url: &str,
    rev: &str,
    dest: &Path,
) -> Result<crate::FetchOutcome, FetchError> {
    // rev flows into `git fetch origin <rev>` as an argument: a value
    // like `--upload-pack=<cmd>` is option injection, not a rev.
    // Shas and plain ref names are all hex/dots/slashes/alnum.
    if !rev
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/'))
        || rev.is_empty()
        || rev.starts_with('-')
    {
        return Err(FetchError::Http {
            url: url.to_string(),
            reason: format!("invalid rev {rev:?}: expected a sha or ref name"),
        });
    }
    let git = |args: &[&str]| {
        let mut command = std::process::Command::new("git");
        command.args(args).current_dir(dest);
        run(&mut command, url).map(|_| ())
    };
    git(&["init", "--quiet"])?;
    git(&["remote", "add", "--", "origin", url])?;
    git(&["fetch", "--quiet", "--depth", "1", "--", "origin", rev])?;
    git(&["checkout", "--quiet", "FETCH_HEAD"])?;
    let mut command = std::process::Command::new("git");
    command
        .args(["rev-parse", "--verify", "HEAD"])
        .current_dir(dest);
    let output = run(&mut command, url)?;
    let commit = String::from_utf8_lossy(&output).trim().to_owned();
    if !matches!(commit.len(), 40 | 64) || !commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(FetchError::Http {
            url: url.into(),
            reason: "git checkout has no valid commit object ID".into(),
        });
    }
    // the checkout is the payload; .git is fetch machinery — pack
    // layout differs per fetch (non-deterministic hash, every apply
    // re-pinned) and it must never reach the store
    std::fs::remove_dir_all(dest.join(".git"))?;
    // the rev is the pin; the tree hash is the payload identity
    super::archive::validate_tree(dest, context.limits())?;
    Ok(crate::FetchOutcome {
        identity: crate::FetchIdentity::Tree(gripsack_store::canonical_tree_hash(dest)?),
        url: None,
        version: Some(commit),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gripsack_ir::FetchSpec;
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
        let context = crate::FetchContext::default();
        let hash = context
            .fetch(
                &FetchSpec::Git {
                    url: remote.to_string_lossy().into_owned(),
                    rev: Some(rev.clone()),
                },
                &dest,
                None,
            )
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(dest.join("file.txt")).unwrap(),
            "v1\n"
        );

        // the checkout is the payload: no .git reaches the store, and
        // the same rev hashes identically on every fetch (a mismatch
        // per fetch made every apply fail its pin check)
        assert!(!dest.join(".git").exists());
        let dest2 = dir.path().join("out2");
        let hash2 = context
            .fetch(
                &FetchSpec::Git {
                    url: remote.to_string_lossy().into_owned(),
                    rev: Some(rev),
                },
                &dest2,
                None,
            )
            .unwrap();
        assert_eq!(
            hash.identity, hash2.identity,
            "same rev must hash identically"
        );
    }
}
