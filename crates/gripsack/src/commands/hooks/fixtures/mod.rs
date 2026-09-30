//! Fixture-only simulations. No repository or live hook is selected. Every
//! worker admits the first-party helper and the exact fixed action digest.
mod receiver;
mod worker;

use gripsack_process::{
    Control, Invocation, Limits, NativeInput, OperatorEnvironment, ProcessRole, Sha256Digest,
};
use gripsack_store as store;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    ffi::{OsStr, OsString},
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    time::Duration,
};

const ROOT_PREFIX: &str = "gripsack-hook-fixture-";
const META: &str = "fixture.json";
const HELPER: &str = "helper";
const MAX_HELPER_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FixtureCase {
    Append,
    Receiver,
}
impl FixtureCase {
    fn name(self) -> &'static str {
        match self {
            Self::Append => "append",
            Self::Receiver => "receiver",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SimulationMode {
    Clean,
    Duplicate,
    CrashAfterStart,
}
impl SimulationMode {
    fn name(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::Duplicate => "duplicate",
            Self::CrashAfterStart => "crash-after-start",
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct FixtureMeta {
    version: u32,
    helper_sha256: Sha256Digest,
    receiver_port: u16,
}

pub(super) struct FixtureRoot {
    path: PathBuf,
    directory: gripsack_fs::Dir,
    meta: FixtureMeta,
}
impl FixtureRoot {
    fn create(path: PathBuf) -> io::Result<Self> {
        let directory = gripsack_fs::open(&path)?;
        let mut source = current_image()?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o500)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path.join(HELPER))?;
        let helper_sha256 = copy_image(&mut source, &mut output)?;
        output.sync_all()?;
        drop(output);
        Ok(Self {
            path,
            directory,
            meta: FixtureMeta {
                version: 1,
                helper_sha256,
                receiver_port: 0,
            },
        })
    }
    fn save(&self) -> io::Result<()> {
        gripsack_fs::atomic_write_with_mode(
            &self.directory,
            Path::new(META),
            &serde_json::to_vec(&self.meta).map_err(io::Error::other)?,
            0o600,
        )
    }
    pub(super) fn open(path: &Path) -> io::Result<Self> {
        if !path.is_absolute()
            || std::fs::canonicalize(path)? != path
            || !path
                .file_name()
                .and_then(OsStr::to_str)
                .is_some_and(|name| name.starts_with(ROOT_PREFIX))
        {
            return Err(invalid(
                "hook simulations require their generated private fixture root",
            ));
        }
        let metadata = std::fs::symlink_metadata(path)?;
        if !metadata.is_dir()
            || metadata.permissions().mode() & 0o7777 != 0o700
            || metadata.uid() != unsafe { libc::geteuid() }
        {
            return Err(invalid(
                "hook fixture root ownership or permissions changed",
            ));
        }
        let directory = gripsack_fs::open(path)?;
        let mut bytes = Vec::new();
        gripsack_fs::open_file_nofollow(&directory, Path::new(META))?
            .take(4097)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 4096 {
            return Err(invalid("hook fixture metadata exceeds its budget"));
        }
        let meta: FixtureMeta = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        if meta.version != 1 || meta.receiver_port == 0 {
            return Err(invalid("unknown or incomplete hook fixture"));
        }
        let mut image = current_image()?;
        if hash_image(&mut image)? != meta.helper_sha256 {
            return Err(invalid("fixture helper is not this first-party executable"));
        }
        Ok(Self {
            path: path.to_owned(),
            directory,
            meta,
        })
    }
    fn helper(&self) -> PathBuf {
        self.path.join(HELPER)
    }
    fn home(&self, case: FixtureCase) -> PathBuf {
        self.path.join(case.name()).join("state")
    }
    fn action(&self, case: FixtureCase) -> io::Result<gripsack_ir::Action> {
        Ok(gripsack_ir::Action::CustomShell {
            script: format!(
                "{} hooks fixture-effect --root {} --case {}",
                quote(&self.helper())?,
                quote(&self.path)?,
                case.name()
            ),
        })
    }
    fn environment(&self, case: FixtureCase) -> io::Result<OperatorEnvironment> {
        let home = self.path.join(case.name());
        let temporary = self.path.join("tmp");
        std::fs::create_dir_all(&home)?;
        std::fs::create_dir_all(&temporary)?;
        OperatorEnvironment::admit([
            (OsString::from("HOME"), home.into_os_string()),
            (OsString::from("TMPDIR"), temporary.into_os_string()),
            (OsString::from("PATH"), OsString::from("/usr/bin:/bin")),
            (OsString::from("LANG"), OsString::from("C")),
        ])
    }
}

#[derive(Serialize)]
pub struct SimulationReport {
    version: u32,
    fixture_only: bool,
    mode: SimulationMode,
    delivery: &'static str,
    failure_policy: &'static str,
    append_effects: usize,
    receiver_effects: u64,
    outcomes: Vec<worker::ObservedIntent>,
}

pub fn simulate(mode: SimulationMode) -> io::Result<SimulationReport> {
    let temporary = tempfile::Builder::new()
        .prefix(ROOT_PREFIX)
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()?;
    let path = std::fs::canonicalize(temporary.path())?;
    let mut root = FixtureRoot::create(path)?;
    let mut receiver = receiver::Receiver::start(&root)?;
    root.meta.receiver_port = receiver.port();
    root.save()?;
    let mut observed = Vec::new();
    for case in [FixtureCase::Append, FixtureCase::Receiver] {
        let environment = root.environment(case)?;
        run_worker(&root, case, mode, &environment)?;
        let before = worker::observe(&root, case)?;
        if mode != SimulationMode::Clean {
            if !before.ambiguous || before.attempt != 1 {
                return Err(invalid("fixture crash did not retain its started attempt"));
            }
            if case == FixtureCase::Receiver {
                // The remote side restarts too: deduplication must come from
                // the atomic token/effect record, not a surviving in-memory set.
                receiver.finish()?;
                receiver = receiver::Receiver::start(&root)?;
                root.meta.receiver_port = receiver.port();
                root.save()?;
            }
            run_worker(&root, case, SimulationMode::Clean, &environment)?;
        }
        let after = worker::observe(&root, case)?;
        let expected_attempt = if mode == SimulationMode::Clean { 1 } else { 2 };
        if !after.succeeded
            || after.pending
            || after.attempt != expected_attempt
            || after.intent != before.intent
        {
            return Err(invalid(
                "fixture replay violated stable identity or durable completion",
            ));
        }
        observed.push(after);
    }
    receiver.finish()?;
    let append = gripsack_fs::open_file_nofollow(&root.directory, Path::new("append.log"))?;
    let mut bytes = Vec::new();
    append.take(4097).read_to_end(&mut bytes)?;
    if bytes.len() > 4096 {
        return Err(invalid("fixture append effects exceed the budget"));
    }
    let append_effects = bytes
        .split(|&byte| byte == b'\n')
        .filter(|line| !line.is_empty())
        .count();
    let expected = if mode == SimulationMode::Duplicate {
        2
    } else {
        1
    };
    let receiver_effects = receiver::effects(&root)?;
    if append_effects != expected || receiver_effects != 1 {
        return Err(invalid(
            "fixture effects did not demonstrate the declared duplicate/deduplication behavior",
        ));
    }
    Ok(SimulationReport {
        version: 1,
        fixture_only: true,
        mode,
        delivery: "at_least_once",
        failure_policy: "warn_no_retry",
        append_effects,
        receiver_effects,
        outcomes: observed,
    })
}

fn run_worker(
    root: &FixtureRoot,
    case: FixtureCase,
    mode: SimulationMode,
    environment: &OperatorEnvironment,
) -> io::Result<()> {
    let timeout = Duration::from_secs(30);
    let deadline = std::time::Instant::now() + timeout;
    let selected = gripsack_process::SelectedProgram::select(
        environment,
        &root.helper(),
        Some(root.meta.helper_sha256),
        deadline,
    )?;
    let invocation = Invocation::admit(
        environment,
        ProcessRole::Probe,
        &selected,
        &root.path,
        Limits {
            timeout,
            operation_deadline: Some(deadline),
            ..Limits::default()
        },
    )?;
    let outcome = invocation.run(
        &[
            OsStr::new("hooks"),
            OsStr::new("fixture-worker"),
            OsStr::new("--root"),
            root.path.as_os_str(),
            OsStr::new("--case"),
            OsStr::new(case.name()),
            OsStr::new("--mode"),
            OsStr::new(mode.name()),
        ],
        NativeInput::Bytes(b""),
        None,
        |_| Control::Continue,
    )?;
    let detail = || {
        let retained = &outcome.stderr[..outcome.stderr.len().min(4096)];
        gripsack_process::terminal::tame(String::from_utf8_lossy(retained).into_owned())
    };
    if mode == SimulationMode::Clean {
        if !outcome.success {
            return Err(invalid(format!(
                "fixture worker failed: {:?}; {}",
                outcome.receipt,
                detail()
            )));
        }
    } else if outcome.receipt.signal != Some(libc::SIGABRT) {
        return Err(invalid(format!(
            "fixture worker did not reach the intentional crash: {:?}; {}",
            outcome.receipt,
            detail()
        )));
    }
    Ok(())
}

pub fn worker(root: &Path, case: FixtureCase, mode: SimulationMode) -> io::Result<()> {
    worker::run(FixtureRoot::open(root)?, case, mode)
}

pub fn effect(root: &Path, case: FixtureCase) -> io::Result<()> {
    let root = FixtureRoot::open(root)?;
    let identity = std::env::var("GRIPSACK_ACTIVATION_INTENT_ID")
        .map_err(|_| invalid("fixture effect has no intent authority"))?;
    let intent = Sha256Digest::parse(&identity)?;
    match case {
        FixtureCase::Append => {
            let path = root.path.join("append.log");
            let mut file = OpenOptions::new()
                .append(true)
                .create(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
                .open(path)?;
            let metadata = file.metadata()?;
            if !metadata.is_file() || metadata.len() > 4096 {
                return Err(invalid(
                    "fixture append target is not an admitted bounded regular file",
                ));
            }
            writeln!(file, "{intent}")?;
            file.sync_all()?;
            gripsack_fs::fsync_dir(&root.directory, Path::new("."))
        }
        FixtureCase::Receiver => receiver::notify(root.meta.receiver_port, intent),
    }
}

fn current_image() -> io::Result<File> {
    #[cfg(target_os = "linux")]
    {
        File::open("/proc/self/exe")
    }
    #[cfg(target_os = "macos")]
    {
        File::open(std::env::current_exe()?)
    }
}
fn hash_image(source: &mut File) -> io::Result<Sha256Digest> {
    copy_image(source, &mut io::sink())
}
fn copy_image(source: &mut File, output: &mut impl Write) -> io::Result<Sha256Digest> {
    let mut hasher = Sha256::new();
    let mut bytes = [0; 32 * 1024];
    let mut total = 0_u64;
    loop {
        let length = source.read(&mut bytes)?;
        if length == 0 {
            break;
        }
        total = total
            .checked_add(length as u64)
            .ok_or_else(|| invalid("fixture executable size overflow"))?;
        if total > MAX_HELPER_BYTES {
            return Err(invalid("fixture executable exceeds its budget"));
        }
        hasher.update(&bytes[..length]);
        output.write_all(&bytes[..length])?;
    }
    Ok(Sha256Digest::from_bytes(hasher.finalize().into()))
}
fn quote(path: &Path) -> io::Result<String> {
    let text = path
        .to_str()
        .ok_or_else(|| invalid("fixture path is not UTF-8"))?;
    Ok(format!("'{}'", text.replace('\'', "'\"'\"'")))
}
fn invalid(message: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
