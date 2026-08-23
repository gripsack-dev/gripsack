//! `git:` — pinned to an immutable rev; a shallow fetch of the exact
//! sha keeps it fast (works on GitHub and any server with
//! allowReachableSHA1InWant).

use super::FetchError;
use std::path::Path;

pub(crate) fn fetch(url: &str, rev: &str, dest: &Path) -> Result<String, FetchError> {
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
    // the rev is the pin; the tree hash is the payload identity
    Ok(gripsack_store::canonical_tree_hash(dest)?)
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
                rev,
            },
            &dest,
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(dest.join("file.txt")).unwrap(),
            "v1\n"
        );
        assert!(!hash.is_empty());
    }
}
