//! Explicit GNU loader invocation preserves a copied program's admitted search
//! plan without trusting its now-invalid memfd-relative $ORIGIN. The loader and
//! application remain separately byte-bound; the application descriptor survives
//! the loader's exec. This does not claim byte binding for shared libraries.
mod capability;
pub use capability::SelectedGnuLoader;
use super::{Image, SelectedProgram};
use crate::{
    Sha256Digest,
    executable::{self, ExecutableFormat, Interpreter},
};
use std::{
    ffi::OsStr,
    io::{self, Read, Seek},
    os::fd::{AsRawFd, RawFd},
    path::Path,
    sync::Arc,
};

pub(super) struct GnuLoader {
    pub selected: Arc<SelectedGnuLoader>,
    pub library_path: std::ffi::OsString,
}
#[derive(Clone, Copy)]
pub(crate) struct ImageDescriptors {
    pub script: Option<RawFd>,
    pub loaded_executable: Option<RawFd>,
}
impl SelectedProgram {
    /// Bind a capability-measured, byte-bound GNU interpreter and the caller's
    /// admitted library plan. Libc release numbers are not loader capabilities:
    /// vendor backports may supply the controls on glibc 2.28.
    pub fn with_gnu_loader(
        mut self,
        loader: Arc<SelectedGnuLoader>,
        library_path: &OsStr,
    ) -> io::Result<Self> {
        if !cfg!(target_os = "linux") {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "explicit GNU loader binding requires Linux",
            ));
        }
        if self.loader.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "an ELF loader is already bound",
            ));
        }
        admit_system_preload(Path::new("/etc/ld.so.preload"))?;
        let mut reader = self.executable.file.try_clone()?;
        reader.rewind()?;
        let metadata = executable::classify(&mut reader)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if !metadata.elf_loader_extensions.is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidData,
                "ELF audit/filter dependencies are outside the admitted GNU loader plan"));
        }
        if metadata.format != Some(ExecutableFormat::Elf)
            || !matches!(&metadata.interpreter,Some(Interpreter::Loader(named)) if Path::new(named) == loader.path())
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "explicit loader differs from the sealed ELF interpreter",
            ));
        }
        self.loader = Some(GnuLoader {
            selected: loader,
            library_path: library_path.to_owned(),
        });
        Ok(self)
    }
    pub(crate) fn execution_image(&self) -> &Image {
        self.loader
            .as_ref()
            .map_or(&self.executable, |loader| &loader.selected.image)
    }
    pub(crate) fn execution_argument_zero(&self) -> &OsStr {
        self.loader
            .as_ref()
            .map_or(self.argument_zero.as_os_str(), |loader| {
                loader.selected.image.path.as_os_str()
            })
    }
    pub(crate) fn loader_arguments(&self) -> impl Iterator<Item = &OsStr> + Clone {
        self.loader
            .as_ref()
            .map(|loader| {
                [
                    OsStr::new("--inhibit-cache"),
                    OsStr::new("--glibc-hwcaps-mask"),
                    OsStr::new(""),
                    OsStr::new("--inhibit-rpath"),
                    OsStr::new(""),
                    OsStr::new("--library-path"),
                    loader.library_path.as_os_str(),
                    OsStr::new("--argv0"),
                    self.argument_zero.as_os_str(),
                    self.executable.path.as_os_str(),
                ]
            })
            .into_iter()
            .flatten()
    }
    pub(crate) fn inherited_images(&self) -> ImageDescriptors {
        if cfg!(target_os = "linux") {
            ImageDescriptors {
                script: self.script.as_ref().map(|image| image.file.as_raw_fd()),
                loaded_executable: self
                    .loader
                    .as_ref()
                    .map(|_| self.executable.file.as_raw_fd()),
            }
        } else {
            ImageDescriptors {
                script: None,
                loaded_executable: None,
            }
        }
    }
}

impl SelectedProgram {
    pub(crate) fn loader_sha256(&self) -> Option<Sha256Digest> {
        self.loader.as_ref().map(|loader| loader.selected.image.digest)
    }

    pub(crate) fn admit_gnu_environment(
        &self,
        entries: &[(std::ffi::OsString, std::ffi::OsString)],
    ) -> io::Result<()> {
        if let Some(loader) = &self.loader {
            for (key, _) in entries {
                loader.selected.check_environment_key(key)?;
            }
        }
        Ok(())
    }
}

/// --inhibit-cache does not suppress /etc/ld.so.preload. Such objects bypass
/// DT_NEEDED admission even with a clean environment, so a nonempty platform
/// preload policy must fail before the selected image can execute.
fn admit_system_preload(path: &Path) -> io::Result<()> {
    const PRELOAD_POLICY_BYTES: u64 = 64 * 1024;
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let status = file.metadata()?;
    if !status.is_file() || status.len() > PRELOAD_POLICY_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "GNU preload policy is not a bounded regular file"));
    }
    let mut bytes = Vec::new();
    file.take(PRELOAD_POLICY_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > PRELOAD_POLICY_BYTES
        || bytes.split(|byte| *byte == b'\n').any(|line| {
            line.split(|byte| *byte == b'#').next().unwrap_or_default()
                .iter().any(|byte| !byte.is_ascii_whitespace())
        })
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "system GNU preload objects are outside the admitted dependency closure",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_preload_cannot_bypass_sanitized_environment() {
        let directory = tempfile::tempdir().unwrap();
        let policy = directory.path().join("ld.so.preload");
        admit_system_preload(&policy).unwrap();
        std::fs::write(&policy, b" \t\n# administrator comment\n").unwrap();
        admit_system_preload(&policy).unwrap();
        for active in ["/tmp/hostile.so\n", "libaudit.so # comment\n", "\0"] {
            std::fs::write(&policy, active).unwrap();
            assert_eq!(admit_system_preload(&policy).unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        }
    }
}
