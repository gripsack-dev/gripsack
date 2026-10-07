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
//! macOS assembles an SBPL profile parent-side and applies it through the
//! fixed platform launcher after exec. Profile parsing and allocation never
//! run in the fork/exec interval. The boundary persists across exec and is
//! inherited by descendants.
//!
//! Platforms without a kernel boundary fail closed: assembling a boundary
//! returns an error and the evaluator refuses to run rather than evaluating
//! unconfined.
#[cfg(target_os = "macos")]
pub(super) mod seatbelt;
mod runtime;
pub use runtime::RuntimeAccess;
#[cfg(target_os = "linux")]
use std::fs::File;
#[cfg(target_os = "linux")]
use std::os::fd::FromRawFd;
#[cfg(target_os = "linux")]
use std::os::unix::io::AsRawFd;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::path::PathBuf;
use std::{io, path::Path};
/// `prctl(PR_SET_NO_NEW_PRIVS)` — fixed as 38 on every Linux ABI.
#[cfg(target_os = "linux")]
const PR_SET_NO_NEW_PRIVS: libc::c_int = 38;

/// The filesystem objects evaluation may touch, assembled before fork.
#[derive(Debug, Default)]
pub struct Boundary {
    read: Vec<PathBuf>,
    read_files: Vec<PathBuf>,
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

    /// Read one existing regular file, never its siblings or parent directory.
    pub fn read_file(mut self, path: &Path) -> io::Result<Self> {
        let canonical = path.canonicalize()?;
        if !std::fs::metadata(&canonical)?.is_file() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput,
                "an exact confinement read grant must name a regular file"));
        }
        if !self.read_files.contains(&canonical) {
            self.read_files.push(canonical);
        }
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

/// An assembled kernel boundary plus the launch-time additions the child
/// must make after fork. Linux holds the populated Landlock ruleset
/// descriptor (per-pid `/proc/self/fd` cannot be anchored parent-side);
/// macOS holds a Seatbelt launch profile, fully assembled while allocation
/// is still allowed and applied by the platform launcher after exec.
pub struct Ruleset {
    #[cfg(target_os = "linux")]
    ruleset: Option<File>,
    #[cfg(target_os = "linux")]
    proc_self_fd: bool,
    #[cfg(target_os = "macos")]
    profile: seatbelt::Profile,
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    _unimplemented: (),
}

// No acknowledgment escape hatch exists: the owner-selected contract (plan/0048,
// 2026-09-29) is fail-closed — hosts without a kernel boundary refuse
// evaluation. Unconfined runs are not qualification for any claim.

// Landlock wire structures (u64 access masks; the kernel parses by size).
#[cfg(target_os = "linux")]
#[repr(C)]
struct RulesetAttr {
    handled_access_fs: u64,
}
#[cfg(target_os = "linux")]
#[repr(C)]
struct PathBeneathAttr {
    allowed_access: u64,
    parent_fd: libc::c_int,
}

#[cfg(target_os = "linux")]
const ACCESS_READ: u64 = LANDLOCK_ACCESS_FS_READ_FILE | LANDLOCK_ACCESS_FS_READ_DIR;
#[cfg(target_os = "linux")]
const ACCESS_READ_WRITE: u64 = ACCESS_READ
    | LANDLOCK_ACCESS_FS_WRITE_FILE
    | LANDLOCK_ACCESS_FS_MAKE_DIR
    | LANDLOCK_ACCESS_FS_MAKE_REG
    | LANDLOCK_ACCESS_FS_MAKE_SYM
    | LANDLOCK_ACCESS_FS_REMOVE_FILE
    | LANDLOCK_ACCESS_FS_REMOVE_DIR;

impl Ruleset {
    /// Duplicate the assembled boundary for one launch. Linux duplicates the
    /// ruleset descriptor (its rules were already added); the Seatbelt
    /// profile is a plain string copy.
    pub(crate) fn try_clone(&self) -> io::Result<Self> {
        #[cfg(target_os = "linux")]
        {
            Ok(Self {
                ruleset: self.ruleset.as_ref().map(File::try_clone).transpose()?,
                proc_self_fd: self.proc_self_fd,
            })
        }
        #[cfg(target_os = "macos")]
        {
            Ok(Self {
                profile: self.profile.clone(),
            })
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            Ok(Self { _unimplemented: () })
        }
    }

    /// A private macOS image's directory joins the boundary for this launch.
    /// The payload and optional script are both private copied images.
    #[cfg(target_os = "macos")]
    pub(crate) fn granting_image_read(self, image: &Path) -> io::Result<Self> {
        Ok(Self {
            profile: self.profile.granting_image(image)?,
        })
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn macos_launch_arguments<'a>(
        &'a self,
        argument_zero: &'a std::ffi::OsStr,
        image: &'a std::ffi::OsStr,
    ) -> [&'a std::ffi::OsStr; 11] {
        self.profile.arguments(argument_zero, image)
    }
}

#[cfg(target_os = "linux")]
mod landlock {
    pub const ACCESS_FS_WRITE_FILE: u64 = 1 << 1;
    pub const ACCESS_FS_READ_FILE: u64 = 1 << 2;
    pub const ACCESS_FS_READ_DIR: u64 = 1 << 3;
    pub const ACCESS_FS_REMOVE_DIR: u64 = 1 << 4;
    pub const ACCESS_FS_REMOVE_FILE: u64 = 1 << 5;
    pub const ACCESS_FS_MAKE_DIR: u64 = 1 << 7;
    pub const ACCESS_FS_MAKE_REG: u64 = 1 << 8;
    pub const ACCESS_FS_MAKE_SYM: u64 = 1 << 12;
    pub const ACCESS_FS_TRUNCATE: u64 = 1 << 14;
    pub const ACCESS_FS_EXECUTE: u64 = 1 << 0;
    pub const RULE_PATH_BENEATH: libc::c_int = 1;
    pub const CREATE_RULESET_VERSION: libc::c_ulong = 1 << 0;
    pub const PROC_SELF_FD: &[u8] = b"/proc/self/fd\0";
}
#[cfg(target_os = "linux")]
use landlock::{
    ACCESS_FS_EXECUTE as LANDLOCK_ACCESS_FS_EXECUTE,
    ACCESS_FS_MAKE_DIR as LANDLOCK_ACCESS_FS_MAKE_DIR,
    ACCESS_FS_MAKE_REG as LANDLOCK_ACCESS_FS_MAKE_REG,
    ACCESS_FS_MAKE_SYM as LANDLOCK_ACCESS_FS_MAKE_SYM,
    ACCESS_FS_READ_DIR as LANDLOCK_ACCESS_FS_READ_DIR,
    ACCESS_FS_READ_FILE as LANDLOCK_ACCESS_FS_READ_FILE,
    ACCESS_FS_REMOVE_DIR as LANDLOCK_ACCESS_FS_REMOVE_DIR,
    ACCESS_FS_REMOVE_FILE as LANDLOCK_ACCESS_FS_REMOVE_FILE,
    ACCESS_FS_TRUNCATE as LANDLOCK_ACCESS_FS_TRUNCATE,
    ACCESS_FS_WRITE_FILE as LANDLOCK_ACCESS_FS_WRITE_FILE,
    CREATE_RULESET_VERSION as LANDLOCK_CREATE_RULESET_VERSION, PROC_SELF_FD,
    RULE_PATH_BENEATH as LANDLOCK_RULE_PATH_BENEATH,
};

impl Ruleset {
    /// Assemble every rule while allocation is still allowed. On Linux
    /// `boundary` itself and the (optional) per-pid `/proc/self/fd`
    /// script-grant are the only launch-time inputs; both are resolved from
    /// fields, never paths. macOS assembles its complete launch profile here;
    /// private image directories are added per launch before payload encoding.
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
            for file in &boundary.read_files {
                ruleset.add_path_beneath(file, LANDLOCK_ACCESS_FS_READ_FILE)?;
            }
            // Executing a binary never widens the read boundary: descendants
            // inherit the read/write rules unchanged, and the evaluator's own
            // process spawning is denied by Deno, not by this ruleset. An
            // operator-selected script runtime may exec its real interpreter
            // anywhere on disk, so EXECUTE alone is granted at the root.
            ruleset.add_path_beneath(Path::new("/"), LANDLOCK_ACCESS_FS_EXECUTE)?;
            Ok(ruleset)
        }
        #[cfg(target_os = "macos")]
        {
            let _ = proc_self_fd;
            Ok(Self {
                profile: seatbelt::Profile::assemble(boundary)?,
            })
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = (boundary, proc_self_fd);
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
        let metadata = anchor.metadata()?;
        let valid = if allowed_access == LANDLOCK_ACCESS_FS_READ_FILE {
            metadata.is_file()
        } else {
            metadata.is_dir()
        };
        if !valid {
            return Err(io::Error::new(io::ErrorKind::InvalidData,
                "confinement anchor changed its admitted object kind"));
        }
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
        let ruleset = self.ruleset.as_ref().expect("assembled linux ruleset");
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

