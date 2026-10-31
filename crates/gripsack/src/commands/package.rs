//! Explicit sharing effects require no repository evaluation or activation.
use gripsack_exec::{Ctx, ExecError, LifecycleSession, Repository, sharing};
use gripsack_process::terminal::tame;
use sharing::registry::{Registry, RegistryAuth, RegistryBundle};
use std::{
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    process::ExitCode,
    sync::Arc,
};

#[derive(Debug, clap::Args)]
pub struct PackageArgs {
    #[command(subcommand)]
    pub action: PackageAction,
}
#[derive(Debug, clap::Subcommand)]
pub enum PackageAction {
    /// Capture explicitly selected native trees and sign a passive bundle.
    Pack {
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long)]
        source: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        key: PathBuf,
        #[arg(long)]
        key_id: String,
    },
    /// Sign exact canonical portable metadata using an explicitly selected seed.
    Sign {
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long)]
        key: PathBuf,
        #[arg(long)]
        key_id: String,
        #[arg(long)]
        output: PathBuf,
    },
    /// Verify and durably import a passive bundle; never activate a profile.
    Import {
        #[arg(long)]
        bundle: PathBuf,
    },
    /// Re-admit current local trust and inspect an imported package.
    Inspect {
        #[arg(long)]
        manifest: String,
    },
    /// Export retained original signed bytes; never copy an installed prefix.
    Export {
        #[arg(long)]
        manifest: String,
        #[arg(long)]
        output: PathBuf,
    },
    /// Publish all blobs before the OCI artifact manifest, explicitly.
    Push {
        #[arg(long)]
        manifest: String,
        #[arg(long)]
        reference: String,
        #[command(flatten)]
        transport: RegistryArgs,
    },
    /// Fetch an exact OCI transport digest and pass it through ordinary import.
    Pull {
        #[arg(long)]
        digest: String,
        #[command(flatten)]
        transport: RegistryArgs,
    },
}
#[derive(Debug, clap::Args)]
pub struct RegistryArgs {
    #[arg(long)]
    registry: String,
    #[arg(long)]
    repository: String,
    /// Anonymous HTTP is permitted only for an explicitly selected loopback registry.
    #[arg(long)]
    insecure_loopback: bool,
    #[arg(long, conflicts_with = "basic_credentials_file")]
    bearer_token_file: Option<PathBuf>,
    #[arg(long, conflicts_with = "bearer_token_file")]
    basic_credentials_file: Option<PathBuf>,
}
fn failure(error: impl std::fmt::Display) -> ExecError {
    std::io::Error::other(error.to_string()).into()
}
pub fn package(args: PackageArgs) -> ExitCode {
    match execute(args) {
        Ok(value) => match serde_json::to_string(&value) {
            Ok(text) => {
                println!("{text}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("grip: {}", tame(error.to_string()));
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            eprintln!("grip: {}", tame(error.to_string()));
            ExitCode::FAILURE
        }
    }
}
fn execute(args: PackageArgs) -> Result<serde_json::Value, ExecError> {
    let home = gripsack_store::gripsack_home();
    let environment = gripsack_process::OperatorEnvironment::capture()?;
    let ctx = Ctx {
        home: home.clone(),
        home_dir: Default::default(),
        repository: Repository::direct(std::env::current_dir()?),
        only: Vec::new(),
        host: gripsack_ir::HostName::parse("package").map_err(ExecError::Gate)?,
        on_progress: None,
        take_over: false,
        take_over_entries: None,
        fetch: Arc::new(gripsack_fetch::FetchContext::from_operator(
            Default::default(),
            &environment,
        )),
    };
    match args.action {
        PackageAction::Sign {
            manifest,
            key,
            key_id,
            output,
        } => {
            let bytes = read_file(&manifest, sharing::format::MAX_MANIFEST_BYTES, false)?;
            let parsed = sharing::format::PortableManifest::decode(&bytes)?;
            let envelope = sharing::trust::sign(&bytes, &key, &key_id).map_err(failure)?;
            create_file(&output, &envelope)?;
            Ok(serde_json::json!({"manifest":parsed.digest()?}))
        }
        PackageAction::Pack {
            manifest,
            source,
            output,
            key,
            key_id,
        } => {
            let specification = read_file(&manifest, sharing::format::MAX_MANIFEST_BYTES, false)?;
            let session = LifecycleSession::acquire(&home)?;
            let digest = sharing::pack(
                &ctx,
                &session,
                &specification,
                &source,
                &output,
                &key,
                &key_id,
            )?;
            Ok(serde_json::json!({"manifest":digest}))
        }
        PackageAction::Inspect { manifest } => {
            let facts = sharing::inspect(&home, &manifest)?
                .ok_or_else(|| failure("package is not imported"))?;
            serde_json::to_value(facts).map_err(Into::into)
        }
        PackageAction::Import { bundle } => {
            let session = LifecycleSession::acquire(&home)?;
            serde_json::to_value(sharing::import_bundle(&ctx, &session, &bundle)?)
                .map_err(Into::into)
        }
        PackageAction::Export { manifest, output } => {
            serde_json::to_value(sharing::export_bundle(&home, &manifest, &output)?)
                .map_err(Into::into)
        }
        PackageAction::Push {
            manifest,
            reference,
            transport,
        } => {
            let registry = registry(transport)?;
            let temporary = tempfile::tempdir()?;
            let bundle = temporary.path().join("bundle");
            sharing::export_bundle(&home, &manifest, &bundle)?;
            let transfer = read_bundle(&bundle)?;
            let transport_digest = registry.push(&reference, transfer).map_err(failure)?;
            Ok(serde_json::json!({"manifest":manifest,"transport_digest":transport_digest}))
        }
        PackageAction::Pull { digest, transport } => {
            let transfer = registry(transport)?.pull(&digest).map_err(failure)?;
            let temporary = tempfile::tempdir()?;
            let bundle = temporary.path();
            transfer.write(bundle)?;
            let session = LifecycleSession::acquire(&home)?;
            serde_json::to_value(sharing::import_bundle(&ctx, &session, bundle)?)
                .map_err(Into::into)
        }
    }
}
fn registry(args: RegistryArgs) -> Result<Registry, ExecError> {
    let auth = if let Some(path) = args.bearer_token_file {
        let bytes = read_file(&path, 16 * 1024, true)?;
        let token = String::from_utf8(bytes).map_err(|_| failure("registry token is not UTF-8"))?;
        RegistryAuth::Bearer(token.trim_end_matches('\n').into())
    } else if let Some(path) = args.basic_credentials_file {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Credentials {
            username: String,
            password: String,
        }
        let credentials: Credentials = serde_json::from_slice(&read_file(&path, 16 * 1024, true)?)
            .map_err(|_| failure("invalid registry credentials file"))?;
        RegistryAuth::Basic {
            username: credentials.username,
            password: credentials.password,
        }
    } else {
        RegistryAuth::Anonymous
    };
    Registry::new(
        &args.registry,
        &args.repository,
        auth,
        args.insecure_loopback,
    )
    .map_err(failure)
}
fn read_bundle(path: &Path) -> Result<RegistryBundle, ExecError> {
    RegistryBundle::read(path, gripsack_fetch::FetchLimits::default())
}
fn read_file(path: &Path, bound: usize, private: bool) -> Result<Vec<u8>, ExecError> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.len() > bound as u64
        || (private
            && (metadata.mode() & 0o777 != 0o600
                || metadata.uid() != rustix::process::getuid().as_raw()))
    {
        return Err(failure(
            "input must be a bounded regular file; credentials require owner-only mode 0600",
        ));
    }
    let mut bytes = Vec::new();
    file.take(bound as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > bound {
        return Err(failure("input exceeds byte bound"));
    }
    Ok(bytes)
}
fn create_file(path: &Path, bytes: &[u8]) -> Result<(), ExecError> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}
