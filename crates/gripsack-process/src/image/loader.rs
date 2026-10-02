//! Explicit GNU loader invocation preserves a copied program's admitted search
//! plan without trusting its now-invalid memfd-relative $ORIGIN. The loader and
//! application remain separately byte-bound; the application descriptor survives
//! the loader's exec. This does not claim byte binding for shared libraries.
use super::{Image, SelectedProgram};
use crate::{
    OperatorEnvironment,
    executable::{self, ExecutableFormat, Interpreter},
};
use std::{
    ffi::OsStr,
    io::{self, Seek},
    os::fd::{AsRawFd, RawFd},
    path::Path,
    time::Instant,
};

pub(super) struct GnuLoader {
    pub image: Image,
    pub library_path: std::ffi::OsString,
}
#[derive(Clone, Copy)]
pub(crate) struct ImageDescriptors {
    pub script: Option<RawFd>,
    pub loaded_executable: Option<RawFd>,
}
impl SelectedProgram {
    /// Bind a GNU ELF interpreter and an already-admitted library search plan.
    /// The caller admits GNU >=2.33, which supplies argv0/hwcaps controls. No
    /// source-selected interpreter substitution or shared-library pin is implied.
    pub fn with_gnu_loader(
        mut self,
        environment: &OperatorEnvironment,
        loader: &Path,
        library_path: &OsStr,
        deadline: Instant,
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
        let mut reader = self.executable.file.try_clone()?;
        reader.rewind()?;
        let metadata = executable::classify(&mut reader)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if metadata.format != Some(ExecutableFormat::Elf)
            || !matches!(&metadata.interpreter,Some(Interpreter::Loader(named)) if Path::new(named) == loader)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "explicit loader differs from the sealed ELF interpreter",
            ));
        }
        let image = Image::copy(&environment.resolve(loader)?, None, deadline)?;
        self.loader = Some(GnuLoader {
            image,
            library_path: library_path.to_owned(),
        });
        Ok(self)
    }
    pub(crate) fn execution_image(&self) -> &Image {
        self.loader
            .as_ref()
            .map_or(&self.executable, |loader| &loader.image)
    }
    pub(crate) fn execution_argument_zero(&self) -> &OsStr {
        self.loader
            .as_ref()
            .map_or(self.argument_zero.as_os_str(), |loader| {
                loader.image.path.as_os_str()
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
