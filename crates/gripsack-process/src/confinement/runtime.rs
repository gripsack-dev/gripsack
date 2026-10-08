//! Exact executable/loader reads, plus explicitly selected interpreter data.
//! Executable identities and grant paths bind approval. System helpers and
//! selected interpreter data are host runtime inputs, never repository inputs.
mod program;
mod query;
use crate::{OperatorEnvironment, ResolvedProgram, executable::Interpreter};
use std::{
    ffi::OsStr,
    io,
    path::{Path, PathBuf},
};

#[derive(Debug, Default, serde::Serialize)]
pub struct RuntimeAccess {
    files: Vec<PathBuf>,
    directories: Vec<PathBuf>,
    script: bool,
    programs: Vec<program::RuntimeProgram>,
}

impl RuntimeAccess {
    pub fn discover(
        environment: &OperatorEnvironment,
        program: &Path,
        mut admit: impl FnMut(&Path) -> io::Result<()>,
    ) -> io::Result<Self> {
        let mut access = Self::default();
        #[cfg(target_os = "macos")]
        {
            super::seatbelt::system_runtime_roots(&mut access.directories);
            for root in &access.directories {
                admit(root)?;
            }
        }
        access.script = access.program(environment, program, &mut admit)?;
        #[cfg(target_os = "linux")]
        match std::fs::symlink_metadata(SYSTEM_LOADER_CACHE) {
            Ok(_) => access.file(Path::new(SYSTEM_LOADER_CACHE), &mut admit)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        Ok(access)
    }

    pub fn files(&self) -> &[PathBuf] {
        &self.files
    }
    pub fn directories(&self) -> &[PathBuf] {
        &self.directories
    }
    pub fn is_script(&self) -> bool {
        self.script
    }

    pub fn primary_digest(&self) -> io::Result<crate::Sha256Digest> {
        self.programs
            .first()
            .map(program::RuntimeProgram::digest)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "runtime program identity is absent",
                )
            })
    }

    pub fn require_selected_identity(&self, identity: &crate::ProgramIdentity) -> io::Result<()> {
        let primary = self.primary_digest()?;
        let matches = if self.script {
            identity.script_sha256 == Some(primary)
                && self
                    .programs
                    .get(1)
                    .is_some_and(|program| program.digest() == identity.executable_sha256)
        } else {
            identity.script_sha256.is_none() && identity.executable_sha256 == primary
        };
        if !matches {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "selected runtime changed during executable/dependency admission",
            ));
        }
        Ok(())
    }

    /// A wrapper's actual Deno engine is an additional named runtime program,
    /// not authority to read every file beside it or along the operator PATH.
    pub fn add_program(
        &mut self,
        environment: &OperatorEnvironment,
        program: &Path,
        mut admit: impl FnMut(&Path) -> io::Result<()>,
    ) -> io::Result<()> {
        self.program(environment, program, &mut admit).map(drop)
    }

    fn program(
        &mut self,
        environment: &OperatorEnvironment,
        program: &Path,
        admit: &mut impl FnMut(&Path) -> io::Result<()>,
    ) -> io::Result<bool> {
        let selected = environment.resolve(program)?;
        admit(selected.declared())?;
        let resolved = selected.canonical();
        admit(resolved)?;
        self.file(resolved, admit)?;
        let (identity, interpreter) = program::measure(&selected)?;
        self.programs.push(identity);
        let Some(Interpreter::Shebang {
            program: interpreter,
            argument,
        }) = interpreter
        else {
            self.libraries(resolved, admit)?;
            return Ok(false);
        };
        let interpreter = environment.resolve(Path::new(&interpreter))?;
        self.interpreter(&interpreter, admit)?;
        let named =
            argument.filter(|_| interpreter.declared().file_name() == Some(OsStr::new("env")));
        let interpreter = if let Some(name) = named {
            let selected = environment.resolve(Path::new(&name))?;
            self.interpreter(&selected, admit)?;
            selected
        } else {
            interpreter
        };
        if interpreter
            .canonical()
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.starts_with("python"))
        {
            self.python_installation(&interpreter, admit)?;
        }
        Ok(true)
    }

    fn interpreter(
        &mut self,
        selected: &ResolvedProgram,
        admit: &mut impl FnMut(&Path) -> io::Result<()>,
    ) -> io::Result<()> {
        admit(selected.declared())?;
        admit(selected.canonical())?;
        let (identity, metadata) = program::measure(selected)?;
        if matches!(metadata, Some(Interpreter::Shebang { .. })) {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "recursive runtime script interpreters are not admitted",
            ));
        }
        self.programs.push(identity);
        self.file(selected.canonical(), admit)?;
        self.libraries(selected.canonical(), admit)
    }

    fn file(
        &mut self,
        path: &Path,
        admit: &mut impl FnMut(&Path) -> io::Result<()>,
    ) -> io::Result<()> {
        admit(path)?;
        let canonical = path.canonicalize()?;
        if !std::fs::metadata(&canonical)?.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "runtime read authority requires a regular file",
            ));
        }
        if !self.files.contains(&canonical) {
            self.files.push(canonical);
        }
        Ok(())
    }

    fn libraries(
        &mut self,
        program: &Path,
        admit: &mut impl FnMut(&Path) -> io::Result<()>,
    ) -> io::Result<()> {
        #[cfg(target_os = "linux")]
        for library in shared_libraries(program)? {
            self.file(&library, admit)?;
        }
        #[cfg(not(target_os = "linux"))]
        let _ = (program, admit);
        Ok(())
    }

    fn python_installation(
        &mut self,
        interpreter: &ResolvedProgram,
        admit: &mut impl FnMut(&Path) -> io::Result<()>,
    ) -> io::Result<()> {
        // The selected spelling, not an ambient python, determines pyvenv.cfg.
        // Its executable digest is bound; stdlib/purelib remain explicit
        // interpreter-installation data rather than repository source.
        let working = tempfile::Builder::new()
            .prefix("gripsack-runtime-probe-")
            .tempdir()?;
        admit(working.path())?;
        let expected = self
            .programs
            .last()
            .map(program::RuntimeProgram::digest)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Python interpreter identity is absent",
                )
            })?;
        // Isolated startup excludes cwd and user-site imports: inspecting
        // a repository must never execute its sysconfig.py before approval.
        let arguments = [
            OsStr::new("-I"),
            OsStr::new("-c"),
            OsStr::new(
                "import sys, sysconfig\nprint(sys.prefix)\np = sysconfig.get_paths()\nprint(p['stdlib'])\nprint(p['purelib'])",
            ),
        ];
        let (outcome, stdout) = query::run(
            interpreter.declared(),
            Some(expected),
            working.path(),
            &arguments,
        )?;
        if !outcome.success {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "selected Python interpreter must support isolated (-I) installation discovery",
            ));
        }
        let text = std::str::from_utf8(&stdout)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let mut lines = text.lines();
        let prefix = lines
            .next()
            .filter(|line| !line.is_empty())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Python runtime prefix is absent",
                )
            })?;
        let prefix = Path::new(prefix);
        if !prefix.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Python runtime prefix is not absolute",
            ));
        }
        let configuration = prefix.join("pyvenv.cfg");
        match std::fs::symlink_metadata(&configuration) {
            Ok(_) => self.file(&configuration, admit)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        // Do not grant sys.prefix itself: bin/ and arbitrary prefix siblings
        // are not interpreter data. Only the selected stdlib and purelib qualify.
        for _ in 0..2 {
            let path = lines
                .next()
                .filter(|line| !line.is_empty())
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Python runtime library path is absent",
                    )
                })?;
            let path = Path::new(path);
            if !path.is_absolute() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Python runtime library path is not absolute",
                ));
            }
            match path.canonicalize() {
                Ok(canonical) if canonical.is_dir() => {
                    admit(path)?;
                    if !self.directories.contains(&canonical) {
                        self.directories.push(canonical);
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Ok(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::NotADirectory,
                        "Python runtime library path is not a directory",
                    ));
                }
                Err(error) => return Err(error),
            }
        }
        if lines.next().is_some() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unexpected Python runtime installation output",
            ));
        }
        Ok(())
    }
}

/// The glibc system loader consults this exact cache, not its parent /etc.
#[cfg(target_os = "linux")]
const SYSTEM_LOADER_CACHE: &str = "/etc/ld.so.cache";

/// Fixed host-tool policy: the OS supplies ldd at these FHS system paths.
/// Never consult operator/repository PATH or inherit loader/Python credentials.
/// Authenticity of these system tools and libraries is the host-integrity trust
/// assumption, not source approval or permission to substitute a repo helper.
#[cfg(target_os = "linux")]
const SYSTEM_LDD: [&str; 2] = ["/usr/bin/ldd", "/bin/ldd"];

#[cfg(target_os = "linux")]
fn shared_libraries(program: &Path) -> io::Result<Vec<PathBuf>> {
    use std::os::unix::fs::PermissionsExt;
    let helper = SYSTEM_LDD.iter().find(|path| std::fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound,
            "cannot derive runtime dependencies: no executable system ldd at /usr/bin/ldd or /bin/ldd"))?;
    let (outcome, bytes) = query::run(
        Path::new(helper),
        None,
        Path::new("/"),
        &[program.as_os_str()],
    )?;
    let stdout = String::from_utf8_lossy(&bytes);
    let stderr = String::from_utf8_lossy(&outcome.stderr);
    if !outcome.success {
        if outcome.receipt.exit_code == Some(1)
            && stdout.lines().chain(stderr.lines()).any(|line| {
                matches!(
                    line.trim(),
                    "not a dynamic executable" | "statically linked"
                )
            })
        {
            return Ok(Vec::new());
        }
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "system ldd could not determine runtime dependencies",
        ));
    }
    if stdout.contains("=> not found") {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "a selected runtime dependency is missing",
        ));
    }
    let mut files = Vec::new();
    for token in stdout
        .split_whitespace()
        .filter(|token| token.starts_with('/'))
    {
        let path = PathBuf::from(token.trim_end_matches([':', ')']));
        let canonical = path.canonicalize()?;
        if !files.contains(&canonical) {
            files.push(canonical);
        }
    }
    Ok(files)
}
