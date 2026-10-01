//! macOS applies Seatbelt after entering the fixed OS launcher, not in a
//! multithreaded parent's post-fork child. The OS launcher and Bash are trusted
//! platform code; neither is selected through repository PATH. Bash executes a
//! constant argv bridge, never repository-supplied shell source.
use super::Boundary;
use std::{
    ffi::{CString, OsStr},
    io,
    os::unix::{ffi::OsStrExt, fs::PermissionsExt},
    path::{Path, PathBuf},
};

pub(crate) const LAUNCHER: &str = "/usr/bin/sandbox-exec";
const ARGUMENT_BRIDGE: &str = "/bin/bash";
const EXEC_BRIDGE: &str = "exec -a \"$1\" \"$2\" \"${@:3}\"";

pub(super) fn system_runtime_roots(roots: &mut Vec<PathBuf>) {
    // Never grant /System itself: /System/Volumes/Data reaches user data.
    // /private/etc and /private/var/db contain credentials, not just runtime
    // support. Only these named system-library/data trees qualify.
    for directory in [
        "/System/Library",
        "/System/Volumes/Preboot/Cryptexes/OS/System/Library",
        "/usr/lib",
        "/usr/share/locale",
        "/usr/share/zoneinfo",
        "/Library/Apple/System/Library",
    ] {
        super::push_existing(Path::new(directory), roots);
    }
}

#[derive(Clone)]
pub(super) struct Profile(CString);
impl Profile {
    pub(super) fn assemble(boundary: &Boundary) -> io::Result<Self> {
        for program in [LAUNCHER, ARGUMENT_BRIDGE] {
            let metadata = std::fs::metadata(program)?;
            if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "a required macOS confinement launcher is not executable",
                ));
            }
        }
        // Landlock parity: metadata queries, executable mapping and process
        // creation remain available. Every descendant inherits the byte-access
        // boundary; Deno independently denies repository subprocess requests.
        let mut profile = String::from(
            "(version 1)\n(deny default)\n\
             (allow signal (target self))\n\
             (allow sysctl-read)\n\
             (allow file-read-metadata)\n\
             (allow file-map-executable)\n\
             (allow process-exec* process-fork)\n\
             (allow file-read* (literal \"/bin/bash\"))\n\
             (allow mach-lookup\n\
             \x20   (global-name \"com.apple.bsd.dirhelper\")\n\
             \x20   (global-name \"com.apple.system.notification_center\"))\n\
             (allow file-read* file-write*\n\
             \x20   (literal \"/dev/null\")\n\
             \x20   (literal \"/dev/zero\")\n\
             \x20   (literal \"/dev/random\")\n\
             \x20   (literal \"/dev/urandom\"))\n",
        );
        for root in &boundary.read {
            if !boundary.read_write.contains(root) {
                read_grant(&mut profile, root)?;
            }
        }
        for root in &boundary.read_write {
            profile.push_str("(allow file-read* file-write* (subpath \"");
            escaped(&mut profile, root)?;
            profile.push_str("\"))\n");
        }
        Ok(Self(CString::new(profile).map_err(invalid_profile)?))
    }
    pub(super) fn granting_image(self, image: &Path) -> io::Result<Self> {
        let directory = image
            .parent()
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "private native image has no parent",
                )
            })?
            .canonicalize()?;
        let mut profile = self
            .0
            .into_string()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        read_grant(&mut profile, &directory)?;
        Ok(Self(CString::new(profile).map_err(invalid_profile)?))
    }
    pub(super) fn arguments<'a>(
        &'a self,
        argument_zero: &'a OsStr,
        image: &'a OsStr,
    ) -> [&'a OsStr; 11] {
        [
            OsStr::new(LAUNCHER),
            OsStr::new("-p"),
            OsStr::from_bytes(self.0.as_bytes()),
            OsStr::new(ARGUMENT_BRIDGE),
            OsStr::new("--noprofile"),
            OsStr::new("--norc"),
            OsStr::new("-c"),
            OsStr::new(EXEC_BRIDGE),
            OsStr::new("gripsack-seatbelt"),
            argument_zero,
            image,
        ]
    }
}
fn invalid_profile(error: std::ffi::NulError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, error)
}
fn read_grant(profile: &mut String, root: &Path) -> io::Result<()> {
    profile.push_str("(allow file-read* (subpath \"");
    escaped(profile, root)?;
    profile.push_str("\"))\n");
    Ok(())
}
fn escaped(profile: &mut String, path: &Path) -> io::Result<()> {
    let path = path.to_str().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "Seatbelt roots must be UTF-8")
    })?;
    for character in path.chars() {
        if matches!(character, '\\' | '"') {
            profile.push('\\');
        }
        profile.push(character);
    }
    Ok(())
}
