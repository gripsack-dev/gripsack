//! Evaluator filesystem confinement: the OS, not Deno's own permission flags,
//! decides which paths evaluation may read or write. Captured-source approval
//! names the bytes that may run; this boundary makes every OTHER filesystem
//! object (ambient `node_modules` ancestors, the operator's home, unrelated
//! temp trees) unreadable by the evaluated process and its descendants.
//!
//! Linux implements Landlock. The ruleset is fully assembled parent-side
//! (`landlock_create_ruleset` + `landlock_add_rule`); the forked child only
//! issues `prctl(PR_SET_NO_PRIVS)` and `landlock_restrict_self` — raw
//! syscalls, no allocation, nothing non-async-signal-safe in `pre_exec`.
//! Restrictions are inherited across `execve` and cannot be dropped by the
//! evaluated program.
//!
//! `EXECUTE` is handled but granted only at the filesystem root: running a
//! binary never widens the read/write boundary (descendants inherit these
//! rules unchanged), and process spawning by evaluated code is denied by
//! Deno's own missing `--allow-run`, not by this ruleset. Mediation stays
//! focused on the R1 property — which bytes may be read or written. Root-wide
//! EXECUTE is what lets an operator-selected script runtime exec its real
//! interpreter at an arbitrary path, and keeps the retained memfd executable
//! binding (`execve` of `/proc/self/fd/N`) working without broad read grants.
//!
//! Other platforms fail closed: assembling a boundary returns an error and the
//! evaluator refuses to run rather than evaluating unconfined.
use std::os::fd::FromRawFd;
use std::{
    ffi::OsStr,
    fs::File,
    io::{self, Read},
    os::unix::io::AsRawFd,
    path::{Path, PathBuf},
    process::Command,
};
/// `prctl(PR_SET_NO_NEW_PRIVS)` — fixed as 38 on every Linux ABI.
const PR_SET_NO_NEW_PRIVS: libc::c_int = 38;

/// The filesystem objects evaluation may touch, assembled before fork.
#[derive(Debug, Default)]
pub struct Boundary {
    read: Vec<PathBuf>,
    read_write: Vec<PathBuf>,
}

impl Boundary {
    pub fn new() -> Self {
        Self::default()
    }

    /// Everything beneath an existing directory becomes readable.
    pub fn read_beneath(mut self, directory: &Path) -> io::Result<Self> {
        push_unique(&mut self.read, directory)?;
        Ok(self)
    }

    /// Everything beneath an existing directory becomes readable, writable,
    /// creatable and removable (the evaluator's private scratch/cache).
    pub fn read_write_beneath(mut self, directory: &Path) -> io::Result<Self> {
        push_unique(&mut self.read_write, directory)?;
        Ok(self)
    }

    /// Like [`Boundary::read_beneath`] but silently skips a root that is not
    /// present on this machine (optional runtime trees such as the provisioned
    /// evaluator home).
    pub fn read_beneath_optional(self, directory: &Path) -> io::Result<Self> {
        match directory.canonicalize() {
            Ok(canonical) if canonical.is_dir() => self.read_beneath(&canonical),
            _ => Ok(self),
        }
    }
}

fn push_unique(roots: &mut Vec<PathBuf>, directory: &Path) -> io::Result<()> {
    let canonical = directory.canonicalize().map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "confinement root {} is unavailable: {error}",
                directory.display()
            ),
        )
    })?;
    if !canonical.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            format!(
                "confinement root {} is not a directory",
                canonical.display()
            ),
        ));
    }
    if !roots.contains(&canonical) {
        roots.push(canonical);
    }
    Ok(())
}

/// An assembled Landlock ruleset plus the launch-time additions the child must
/// make after fork (per-pid `/proc/self/fd` cannot be anchored parent-side).
pub struct Ruleset {
    ruleset: Option<File>,
    proc_self_fd: bool,
}

/// Operator-acknowledged unconfinement for platforms with no kernel boundary
/// yet (macOS today). The default stays fail-closed; only this explicit
/// operator environment variable — never a repository-controlled value —
/// lets evaluation run, and every launch says so on stderr.
#[cfg(not(target_os = "linux"))]
pub const UNCONFINED_ACKNOWLEDGMENT: &str = "GRIPSACK_EVAL_UNCONFINED";

// Landlock wire structures (u64 access masks; the kernel parses by size).
#[repr(C)]
struct RulesetAttr {
    handled_access_fs: u64,
}
#[repr(C)]
struct PathBeneathAttr {
    allowed_access: u64,
    parent_fd: libc::c_int,
}

const ACCESS_READ: u64 = LANDLOCK_ACCESS_FS_READ_FILE | LANDLOCK_ACCESS_FS_READ_DIR;
const ACCESS_READ_WRITE: u64 = ACCESS_READ
    | LANDLOCK_ACCESS_FS_WRITE_FILE
    | LANDLOCK_ACCESS_FS_MAKE_DIR
    | LANDLOCK_ACCESS_FS_MAKE_REG
    | LANDLOCK_ACCESS_FS_MAKE_SYM
    | LANDLOCK_ACCESS_FS_REMOVE_FILE
    | LANDLOCK_ACCESS_FS_REMOVE_DIR;

impl Ruleset {
    /// Duplicate the assembled ruleset descriptor for one launch. Rules were
    /// already added; the duplicate behaves identically for restriction.
    pub(crate) fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            ruleset: self.ruleset.as_ref().map(File::try_clone).transpose()?,
            proc_self_fd: self.proc_self_fd,
        })
    }
}

const LANDLOCK_ACCESS_FS_WRITE_FILE: u64 = 1 << 1;
const LANDLOCK_ACCESS_FS_READ_FILE: u64 = 1 << 2;
const LANDLOCK_ACCESS_FS_READ_DIR: u64 = 1 << 3;
const LANDLOCK_ACCESS_FS_REMOVE_DIR: u64 = 1 << 4;
const LANDLOCK_ACCESS_FS_REMOVE_FILE: u64 = 1 << 5;
const LANDLOCK_ACCESS_FS_MAKE_DIR: u64 = 1 << 7;
const LANDLOCK_ACCESS_FS_MAKE_REG: u64 = 1 << 8;
const LANDLOCK_ACCESS_FS_MAKE_SYM: u64 = 1 << 12;
const LANDLOCK_ACCESS_FS_TRUNCATE: u64 = 1 << 14;
const LANDLOCK_ACCESS_FS_EXECUTE: u64 = 1 << 0;

const LANDLOCK_RULE_PATH_BENEATH: libc::c_int = 1;
const LANDLOCK_CREATE_RULESET_VERSION: libc::c_ulong = 1 << 0;
const PROC_SELF_FD: &[u8] = b"/proc/self/fd\0";

impl Ruleset {
    /// Assemble every rule while allocation is still allowed. `boundary`
    /// itself and the (optional) per-pid `/proc/self/fd` script-grant are the
    /// only launch-time inputs; both are resolved from fields, never paths.
    pub fn assemble(boundary: &Boundary, proc_self_fd: bool) -> io::Result<Self> {
        #[cfg(target_os = "linux")]
        {
            let abi = landlock_abi()?;
            let mut handled = ACCESS_READ_WRITE;
            let truncate = ACCESS_READ_WRITE | LANDLOCK_ACCESS_FS_TRUNCATE;
            let read_write_access = if abi >= 3 {
                truncate
            } else {
                ACCESS_READ_WRITE
            };
            if abi >= 3 {
                handled = truncate;
            }
            handled |= LANDLOCK_ACCESS_FS_EXECUTE;
            let attr = RulesetAttr {
                handled_access_fs: handled,
            };
            // SAFETY: attr is a live properly-sized local; the returned fd on
            // success is a new owned descriptor wrapped immediately.
            let fd = unsafe {
                libc::syscall(
                    libc::SYS_landlock_create_ruleset,
                    &attr,
                    std::mem::size_of::<RulesetAttr>(),
                    0 as libc::c_ulong,
                )
            };
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: success yields a fresh owned descriptor.
            let ruleset = unsafe { File::from_raw_fd(fd as libc::c_int) };
            let ruleset = Self {
                ruleset: Some(ruleset),
                proc_self_fd,
            };
            for root in boundary.read.iter().chain(&boundary.read_write) {
                let access = if boundary.read_write.contains(root) {
                    read_write_access
                } else {
                    ACCESS_READ
                };
                ruleset.add_path_beneath(root, access)?;
            }
            // Executing a binary never widens the read boundary: descendants
            // inherit the read/write rules unchanged, and the evaluator's own
            // process spawning is denied by Deno, not by this ruleset. An
            // operator-selected script runtime may exec its real interpreter
            // anywhere on disk, so EXECUTE alone is granted at the root.
            ruleset.add_path_beneath(Path::new("/"), LANDLOCK_ACCESS_FS_EXECUTE)?;
            Ok(ruleset)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (boundary, proc_self_fd);
            if std::env::var_os(UNCONFINED_ACKNOWLEDGMENT).is_some_and(|value| value == "1") {
                eprintln!(
                    "grip: no evaluator filesystem confinement exists on this platform; \
                     running unconfined per {UNCONFINED_ACKNOWLEDGMENT}=1 (operator-acknowledged)"
                );
                return Ok(Self {
                    ruleset: None,
                    proc_self_fd: false,
                });
            }
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "evaluator filesystem confinement is not implemented on this platform; \
                 evaluation is refused rather than run unconfined",
            ))
        }
    }

    #[cfg(target_os = "linux")]
    fn add_path_beneath(&self, directory: &Path, allowed_access: u64) -> io::Result<()> {
        use std::os::unix::fs::OpenOptionsExt;
        let anchor = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_PATH | libc::O_CLOEXEC)
            .open(directory)?;
        let attr = PathBeneathAttr {
            allowed_access,
            parent_fd: anchor.as_raw_fd(),
        };
        // SAFETY: attr refers to the live anchor descriptor for this call.
        let status = unsafe {
            libc::syscall(
                libc::SYS_landlock_add_rule,
                self.ruleset
                    .as_ref()
                    .expect("assembled linux ruleset")
                    .as_raw_fd(),
                LANDLOCK_RULE_PATH_BENEATH,
                &attr,
                0 as libc::c_ulong,
            )
        };
        if status < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Apply the assembled ruleset to the calling thread. Only raw syscalls:
    /// an optional `/proc/self/fd` read rule (this pid's directory cannot be
    /// anchored by the parent), `PR_SET_NO_PRIVS`, `restrict_self`.
    #[cfg(target_os = "linux")]
    pub(crate) fn restrict(&self) -> io::Result<()> {
        let Some(ruleset) = self.ruleset.as_ref() else {
            return Ok(()); // acknowledged-unconfined platform placeholder
        };
        if self.proc_self_fd {
            // SAFETY: static NUL-terminated path; success yields an owned fd.
            let fd =
                unsafe { libc::open(PROC_SELF_FD.as_ptr().cast(), libc::O_PATH | libc::O_CLOEXEC) };
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            let attr = PathBeneathAttr {
                allowed_access: ACCESS_READ,
                parent_fd: fd,
            };
            // SAFETY: attr refers to the just-opened descriptor.
            let status = unsafe {
                libc::syscall(
                    libc::SYS_landlock_add_rule,
                    ruleset.as_raw_fd(),
                    LANDLOCK_RULE_PATH_BENEATH,
                    &attr,
                    0 as libc::c_ulong,
                )
            };
            let error = if status < 0 {
                Some(io::Error::last_os_error())
            } else {
                None
            };
            // SAFETY: an integer descriptor this scope owns.
            unsafe { libc::close(fd) };
            if let Some(error) = error {
                return Err(error);
            }
        }
        // SAFETY: scalar prctl arguments.
        if unsafe { libc::prctl(PR_SET_NO_NEW_PRIVS, 1 as libc::c_ulong, 0, 0, 0) } < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the owned ruleset descriptor and a zero flags word.
        let status = unsafe {
            libc::syscall(
                libc::SYS_landlock_restrict_self,
                ruleset.as_raw_fd(),
                0 as libc::c_ulong,
            )
        };
        if status < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    pub(crate) fn restrict(&self) -> io::Result<()> {
        Ok(()) // only the acknowledged-unconfined placeholder exists here
    }
}

#[cfg(target_os = "linux")]
fn landlock_abi() -> io::Result<u32> {
    // SAFETY: version queries pass no structures.
    let abi = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            std::ptr::null::<RulesetAttr>(),
            0_usize,
            LANDLOCK_CREATE_RULESET_VERSION,
        )
    };
    if abi < 0 {
        let error = io::Error::last_os_error();
        return Err(io::Error::new(
            error.kind(),
            format!(
                "kernel filesystem isolation is unavailable ({error}); \
                 evaluation is refused rather than run unconfined"
            ),
        ));
    }
    Ok(abi as u32)
}

/// Read roots the selected runtime itself needs: the program's directory, its
/// dynamic loader dependencies (`ldd`, operator-trusted input only), any `#!`
/// interpreter with the same treatment, and a python interpreter's standard
/// library when the runtime is a script wrapper. `EXECUTE` needs no rule; see
/// the module header.
#[cfg(target_os = "linux")]
pub fn runtime_read_roots(
    environment: &super::OperatorEnvironment,
    program: &Path,
) -> io::Result<Vec<PathBuf>> {
    let mut roots = Vec::new();
    let resolved = resolve_program(environment, program)?;
    match script_interpreter(&resolved)? {
        None => add_program_roots(&resolved, &mut roots)?,
        Some((interpreter, argument)) => {
            // `#!/usr/bin/env NAME` resolves NAME through the operator PATH;
            // the named program — not `env` — supplies the real load roots.
            let named = argument.filter(|_| {
                Path::new(&interpreter)
                    .file_name()
                    .and_then(OsStr::to_str)
                    .is_some_and(|name| name == "env")
            });
            add_program_roots(&interpreter, &mut roots)?;
            let (resolved, origin) = match named {
                Some(name) => (resolve_program(environment, Path::new(&name))?, name),
                None => {
                    let origin = interpreter
                        .file_name()
                        .and_then(OsStr::to_str)
                        .unwrap_or_default()
                        .to_owned();
                    (interpreter, origin)
                }
            };
            add_program_roots(&resolved, &mut roots)?;
            if is_python(&resolved) {
                add_python_stdlib(&resolved, &origin, &mut roots);
            }
        }
    }
    Ok(roots)
}

/// Without a kernel confinement mechanism there is no boundary to derive
/// roots for; evaluator launch fails closed at ruleset assembly instead.
#[cfg(not(target_os = "linux"))]
pub fn runtime_read_roots(
    _environment: &super::OperatorEnvironment,
    _program: &Path,
) -> io::Result<Vec<PathBuf>> {
    Ok(Vec::new())
}

#[cfg(target_os = "linux")]
fn resolve_program(
    environment: &super::OperatorEnvironment,
    program: &Path,
) -> io::Result<PathBuf> {
    environment.resolve(program)
}

#[cfg(target_os = "linux")]
fn add_program_roots(resolved: &Path, roots: &mut Vec<PathBuf>) -> io::Result<()> {
    let parent = resolved.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "selected runtime has no parent directory",
        )
    })?;
    push_existing(parent, roots);
    for library in shared_libraries(resolved)? {
        if let Some(parent) = library.parent() {
            push_existing(parent, roots);
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn push_existing(directory: &Path, roots: &mut Vec<PathBuf>) {
    if let Ok(canonical) = directory.canonicalize()
        && canonical.is_dir()
        && !roots.contains(&canonical)
    {
        roots.push(canonical);
    }
}

/// `ldd` over an operator-selected program (never repository bytes). Missing
/// `ldd` is a hard error: without library roots the confined runtime could
/// not start, and guessing would either over-grant or under-grant silently.
#[cfg(target_os = "linux")]
fn shared_libraries(resolved: &Path) -> io::Result<Vec<PathBuf>> {
    let output = Command::new("ldd")
        .arg(resolved)
        .output()
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "cannot derive runtime library roots (ldd unavailable): {error}; \
                 evaluation is refused rather than run with guessed grants"
                ),
            )
        })?;
    if !output.status.success() {
        // Static executables ("not a dynamic executable") legitimately have
        // no library roots. Any other ldd refusal yields no roots too: a
        // dynamic binary we could not map then fails closed at exec with a
        // visible access error rather than running under guessed grants.
        return Ok(Vec::new());
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut libraries = Vec::new();
    for token in text.split_whitespace() {
        if !token.starts_with('/') {
            continue;
        }
        let path = PathBuf::from(token.trim_end_matches([':', ')']));
        // "not found" targets and informational maps are skipped by existence.
        if path.exists() {
            libraries.push(path);
        }
    }
    Ok(libraries)
}

#[cfg(target_os = "linux")]
fn script_interpreter(resolved: &Path) -> io::Result<Option<(PathBuf, Option<String>)>> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(resolved)?;
    let mut header = [0; 128];
    let length = file.read(&mut header)?;
    if length < 2 || &header[..2] != b"#!" {
        return Ok(None);
    }
    let line = header[..length]
        .split(|&byte| byte == b'\n')
        .next()
        .unwrap_or(&[]);
    let line = line.trim_ascii();
    let text = String::from_utf8_lossy(&line[2..]);
    let mut parts = text.splitn(2, [' ', '\t']);
    let Some(interpreter) = parts.next().filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let argument = parts
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let interpreter = PathBuf::from(interpreter);
    if !interpreter.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "selected runtime script interpreter must be absolute",
        ));
    }
    Ok(Some((interpreter, argument.map(str::to_owned))))
}

#[cfg(target_os = "linux")]
fn is_python(resolved: &Path) -> bool {
    resolved
        .file_name()
        .and_then(OsStr::to_str)
        .is_some_and(|name| name.starts_with("python"))
}

#[cfg(target_os = "linux")]
fn add_python_stdlib(interpreter: &Path, origin: &str, roots: &mut Vec<PathBuf>) {
    use std::os::unix::process::CommandExt;
    let mut command = Command::new(interpreter);
    // The confined interpreter resolves its own venv from argv[0] through the
    // operator PATH; the probe must perform that same walk to observe it.
    if !origin.is_empty() {
        command.arg0(origin);
    }
    let output = command
        .args(["-E", "-c"])
        .arg(
            "import sys, sysconfig\n\
             print(sys.prefix)\n\
             paths = sysconfig.get_paths()\n\
             print(paths['stdlib'])\n\
             print(paths['purelib'])",
        )
        .output();
    let Ok(output) = output else { return };
    if !output.status.success() {
        return;
    }
    // The prefix line covers a virtualenv interpreter's own tree (pyvenv.cfg,
    // site-packages); the sysconfig lines cover the base standard library.
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        push_existing(Path::new(line), roots);
    }
}
