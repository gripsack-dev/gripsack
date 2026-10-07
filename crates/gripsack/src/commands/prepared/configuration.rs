//! Bootstrap selection is data only, bounded and checked against copied bytes.
use super::{map_diagnostics, operational};
use crate::render::DiagnosticSink;
use gripsack_process::Sha256Digest;
use gripsack_store::source_bundle::SourceBundle;
use std::{io::{self, Read}, path::Path, process::ExitCode};

/// Configuration has a smaller bootstrap budget than arbitrary source files.
const MAX_CONFIGURATION_BYTES: u64 = 1024 * 1024;

pub(super) struct Configuration {
    pub env: gripsack_config::EnvConfig,
    digest: Option<Sha256Digest>,
}

impl Configuration {
    pub fn read(
        repo: &Path,
        sources: Option<&SourceBundle>,
        sink: &mut DiagnosticSink,
    ) -> Result<Self, ExitCode> {
        let directory = gripsack_fs::open(repo).map_err(operational)?;
        let metadata = match directory.symlink_metadata("env.toml") {
            Ok(metadata) => Some(metadata),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(operational(error)),
        };
        if let Some(metadata) = metadata {
            if !metadata.is_file() || metadata.len() > MAX_CONFIGURATION_BYTES {
                return Err(operational(io::Error::other(
                    "env.toml must be a regular non-symlink file within its 1 MiB bootstrap budget",
                )));
            }
            let file = gripsack_fs::open_file_nofollow(&directory, Path::new("env.toml"))
                .map_err(operational)?;
            let mut bytes = Vec::new();
            file.take(MAX_CONFIGURATION_BYTES + 1).read_to_end(&mut bytes).map_err(operational)?;
            if bytes.len() as u64 > MAX_CONFIGURATION_BYTES {
                return Err(operational(io::Error::other("env.toml exceeds its bootstrap byte budget")));
            }
            let digest = Some(Sha256Digest::of(&bytes));
            let source = std::str::from_utf8(&bytes).map_err(|error| operational(io::Error::other(error)))?;
            let env = gripsack_config::parse_env(source).map_err(|mut diagnostics| {
                if let Some(sources) = sources {
                    map_diagnostics(sources, &mut diagnostics);
                }
                sink.report(&diagnostics);
                ExitCode::FAILURE
            })?;
            return Ok(Self { env, digest });
        }
        match directory.symlink_metadata("gripsack.ts") {
            Ok(metadata) if metadata.is_file() || metadata.file_type().is_symlink() => Ok(Self {
                env: gripsack_config::EnvConfig::default(),
                digest: None,
            }),
            Ok(_) => Err(operational(io::Error::other("gripsack.ts is not a regular workspace source"))),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Err(operational(io::Error::other(
                "no env.toml or gripsack.ts in the repository",
            ))),
            Err(error) => Err(operational(error)),
        }
    }

    pub fn require_same(&self, captured: &Self) -> io::Result<()> {
        if self.digest != captured.digest {
            return Err(io::Error::new(io::ErrorKind::InvalidData,
                "env.toml changed during source capture; inspect the source and capture policy again"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_bootstrap_configuration_cannot_authorize_the_captured_tree() {
        use gripsack_store::source_bundle::SourceCapturePolicy;
        let temporary = tempfile::tempdir().unwrap();
        let repo = temporary.path().join("repo");
        let frontend = temporary.path().join("frontend");
        std::fs::create_dir(&repo).unwrap();
        std::fs::create_dir(&frontend).unwrap();
        std::fs::write(repo.join("gripsack.ts"), "// entrypoint").unwrap();
        let config = repo.join("env.toml");
        for replacement in [
            Some("[capture]\nexclude = []\n"),
            None,
        ] {
            std::fs::write(&config, "[capture]\nexclude = ['.venv']\n").unwrap();
            let initial = Configuration::read(&repo, None, &mut DiagnosticSink::json()).unwrap();
            let policy = SourceCapturePolicy::new(initial.env.capture.exclude.clone()).unwrap();
            if let Some(replacement) = replacement {
                std::fs::write(&config, replacement).unwrap();
            } else {
                std::fs::remove_file(&config).unwrap();
            }
            let bundle = SourceBundle::capture(&repo, &frontend, None, &temporary.path().join("runtime"), policy.clone()).unwrap();
            let captured = Configuration::read(
                bundle.repository(), Some(&bundle), &mut DiagnosticSink::json(),
            ).unwrap();
            assert!(initial.require_same(&captured).is_err());
            assert!(captured.require_same(&initial).is_err());
            assert!(captured.require_same(&captured).is_ok());
        }
    }
}
