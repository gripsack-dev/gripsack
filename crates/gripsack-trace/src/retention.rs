//! Diagnostic logs only. The directory lock serializes allocation, `latest`
//! publication and pruning. A separate lock on each log spans the command;
//! neither lock requires a deletable lockfile or a PID/liveness guess.
use gripsack_host::fs::{Dir, cap_std::fs::MetadataExt as _};
use rustix::fs::{self, Mode, OFlags, ResolveFlags};
use rustix::io::Errno;
use rustix::process::geteuid;
use std::ffi::OsStr;
use std::fs::{File, Permissions};
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

const KEEP: usize = 100;
const NO_ALIASES: ResolveFlags = ResolveFlags::NO_SYMLINKS.union(ResolveFlags::NO_XDEV);

#[derive(Debug)]
pub(super) struct Session {
    runs: Dir,
    pub(super) file: Arc<File>,
}

fn private_runs(home: &Path) -> io::Result<Dir> {
    let home = gripsack_host::fs::open_or_create(home)?;
    match fs::mkdirat(&home, "runs", Mode::RWXU) {
        Ok(()) | Err(Errno::EXIST) => {}
        Err(e) => return Err(e.into()),
    }
    // O_PATH can pin even a mode-000 directory (e.g. umask 0777). Never
    // chmod the ambient path, a foreign directory, a symlink or a bind mount.
    let pinned = fs::openat2(
        &home,
        "runs",
        OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
        NO_ALIASES,
    )?;
    let stat = fs::fstat(&pinned)?;
    if stat.st_uid != geteuid().as_raw() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "foreign runs directory",
        ));
    }
    if stat.st_mode & 0o7777 != 0o700 {
        std::fs::set_permissions(
            format!("/proc/self/fd/{}", pinned.as_raw_fd()),
            Permissions::from_mode(0o700),
        )?;
    }
    gripsack_host::fs::fsync_dir(&home, Path::new("."))?;
    Ok(Dir::from_std_file(File::from(fs::openat(
        &pinned,
        ".",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )?)))
}

fn lock_directory(runs: &Dir) -> io::Result<File> {
    // Independent open description: dropping this file releases only this lock.
    let file = File::from(fs::openat(
        runs,
        ".",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )?);
    file.lock()?;
    Ok(file)
}

fn new_id() -> io::Result<String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_nanos();
    let mut entropy = [0u8; 4];
    File::open("/dev/urandom")?.read_exact(&mut entropy)?;
    Ok(format!(
        "run-{nanos:032x}-{:08x}",
        u32::from_le_bytes(entropy)
    ))
}

fn managed_id(name: &OsStr) -> Option<&str> {
    let id = name.to_str()?.strip_suffix(".jsonl")?;
    let (time, salt) = id.strip_prefix("run-")?.split_once('-')?;
    (time.len() == 32
        && salt.len() == 8
        && time
            .bytes()
            .chain(salt.bytes())
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()))
    .then_some(id)
}

fn allocate(
    runs: &Dir,
    mut make_id: impl FnMut() -> io::Result<String>,
) -> io::Result<(String, File)> {
    // A reserved pointer may replace another symlink, never somebody's file.
    match runs.symlink_metadata("latest") {
        Ok(m) if m.is_symlink() && m.uid() == geteuid().as_raw() => {}
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "foreign runs/latest",
            ));
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    for _ in 0..16 {
        let id = make_id()?;
        let name = format!("{id}.jsonl");
        let mode = Mode::RUSR | Mode::WUSR;
        let flags =
            OFlags::CREATE | OFlags::EXCL | OFlags::WRONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW;
        let fd = match fs::openat(runs, name.as_str(), flags, mode) {
            Ok(fd) => fd,
            Err(Errno::EXIST) => continue,
            Err(e) => return Err(e.into()),
        };
        let file = File::from(fd);
        fs::fchmod(&file, mode)?;
        file.lock()?;
        // Unlike a replace helper that pre-unlinks its temporary name, never
        // unlink a colliding/foreign temporary entry, even after a crash.
        let temp = format!(".{id}.latest");
        if let Err(e) = fs::symlinkat(name.as_str(), runs, temp.as_str()) {
            runs.remove_file(&name)?;
            if e == Errno::EXIST {
                continue;
            }
            return Err(e.into());
        }
        if let Err(e) = runs.rename(&temp, runs, "latest") {
            let _ = runs.remove_file(&temp);
            let _ = runs.remove_file(&name);
            return Err(e);
        }
        // Keep the established publication durability of the latest pointer.
        gripsack_host::fs::fsync_dir(runs, Path::new("."))?;
        return Ok((id, file));
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "run ID collisions",
    ))
}

pub(super) fn start(home: &Path) -> io::Result<(String, Session)> {
    let runs = private_runs(home)?;
    let _lock = lock_directory(&runs)?;
    let (id, file) = allocate(&runs, new_id)?;
    let _ = prune(&runs);
    Ok((
        id,
        Session {
            runs,
            file: Arc::new(file),
        },
    ))
}

impl Drop for Session {
    fn drop(&mut self) {
        // A killed process cannot run this pass, but its file lock is released
        // by the kernel and its log is considered on the next pass.
        if let Ok(_lock) = lock_directory(&self.runs)
            && self.file.unlock().is_ok()
        {
            let _ = prune(&self.runs);
        }
    }
}

fn prune(runs: &Dir) -> io::Result<()> {
    let mut names = Vec::new();
    for entry in runs.entries()? {
        let entry = entry?;
        let name = entry.file_name();
        if managed_id(&name).is_some() && entry.file_type()?.is_file() {
            names.push(name);
        }
    }
    if names.len() <= KEEP {
        return Ok(());
    }
    // Fixed-width start time, then salt: deterministic even at identical times.
    names.sort_unstable_by(|a, b| b.cmp(a));
    let latest = runs.read_link_contents("latest")?;
    let mut completed = 0;
    for name in names {
        let Some(_file) = completed_log(runs, &name) else {
            continue;
        };
        completed += 1;
        if completed > KEEP && latest.file_name() != Some(name.as_os_str()) {
            runs.remove_file(name)?;
        }
    }
    Ok(())
}

#[derive(serde::Deserialize)]
struct Opening<'a> {
    target: &'a str,
    #[serde(borrow)]
    fields: OpeningFields<'a>,
}

#[derive(serde::Deserialize)]
struct OpeningFields<'a> {
    message: &'a str,
    run_id: &'a str,
}

fn completed_log(runs: &Dir, name: &OsStr) -> Option<File> {
    let id = managed_id(name)?;
    let fd = fs::openat2(
        runs,
        name,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NONBLOCK | OFlags::NOCTTY,
        Mode::empty(),
        NO_ALIASES,
    )
    .ok()?;
    let mut file = File::from(fd);
    let meta = file.metadata().ok()?;
    if !meta.is_file()
        || meta.nlink() != 1
        || meta.uid() != geteuid().as_raw()
        || file.try_lock().is_err()
    {
        return None;
    }
    // A matching filename alone does not establish ownership. Only this
    // writer emits the named opening event as its first record. Don't read or
    // allocate proportional to an arbitrary foreign file or unterminated line.
    let mut prefix = [0u8; 512];
    let n = file.read(&mut prefix).ok()?;
    let end = prefix[..n].iter().position(|&b| b == b'\n')?;
    let event: Opening<'_> = serde_json::from_slice(&prefix[..end]).ok()?;
    (event.target == "gripsack_trace"
        && event.fields.message == "run log opened"
        && event.fields.run_id == id)
        .then_some(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collisions_never_truncate_logs_or_remove_foreign_temporary_entries() {
        let home = tempfile::tempdir().unwrap();
        let runs = private_runs(home.path()).unwrap();
        runs.write("first.jsonl", b"keep log").unwrap();
        runs.write(".second.latest", b"keep temp").unwrap();
        let mut ids = ["first", "second", "third"].into_iter();
        let (id, _file) = allocate(&runs, || Ok(ids.next().unwrap().into())).unwrap();
        assert_eq!(id, "third");
        assert_eq!(runs.read("first.jsonl").unwrap(), b"keep log");
        assert_eq!(runs.read(".second.latest").unwrap(), b"keep temp");
        assert!(!runs.exists("second.jsonl"));
        assert_eq!(
            runs.read_link_contents("latest").unwrap(),
            Path::new("third.jsonl")
        );
    }
}
