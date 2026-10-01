//! Snapshot operator-owned native inputs before a repository overlay is read.
//! No ambient credential or loader-variable inheritance is implicit.
use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    io,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessRole {
    Evaluator,
    Probe,
    Build,
    Plugin,
    Verify,
    Hook,
    Fact,
    Update,
    /// A project consumer process (run/shell/task): repository-selected
    /// commands on the host, with an explicit repository overlay and no
    /// evaluator-private state. Interactive admission is exclusive to it.
    Task,
}

#[derive(Debug, Clone)]
pub struct OperatorEnvironment {
    values: BTreeMap<OsString, OsString>,
    search_path: Vec<PathBuf>,
}

impl OperatorEnvironment {
    pub fn capture() -> io::Result<Self> {
        Self::admit(std::env::vars_os())
    }

    /// Admission is also used by isolated fixtures: supplied values are the
    /// operator snapshot, not a repository-controlled replacement environment.
    pub fn admit(values: impl IntoIterator<Item = (OsString, OsString)>) -> io::Result<Self> {
        let mut admitted = BTreeMap::new();
        for (key, value) in values {
            let Some(name) = key.to_str() else {
                continue;
            };
            if operator_key(name) {
                admitted.insert(key, value);
            }
        }
        let path = admitted
            .entry(OsString::from("PATH"))
            .or_insert_with(|| OsString::from("/usr/bin:/bin"));
        let search_path: Vec<_> = std::env::split_paths(path).collect();
        if search_path.iter().any(|entry| !entry.is_absolute()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "native process PATH contains an empty or relative search directory",
            ));
        }
        Ok(Self {
            values: admitted,
            search_path,
        })
    }

    /// The evaluator's cache is a core-selected runtime directory, never an
    /// ambient or repository-controlled DENO_DIR.
    pub fn with_evaluator_cache(mut self, directory: &Path) -> io::Result<Self> {
        if !directory.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "evaluator cache must be absolute",
            ));
        }
        self.values
            .insert(OsString::from("DENO_DIR"), directory.as_os_str().to_owned());
        Ok(self)
    }

    /// The evaluator's scratch directory replaces any ambient TMPDIR/TMP/TEMP
    /// so the confined runtime cannot reach operator temp trees.
    pub fn with_evaluator_temp(mut self, directory: &Path) -> io::Result<Self> {
        if !directory.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "evaluator temp directory must be absolute",
            ));
        }
        self.values
            .insert(OsString::from("TMPDIR"), directory.as_os_str().to_owned());
        self.values.remove(OsStr::new("TMP"));
        self.values.remove(OsStr::new("TEMP"));
        Ok(self)
    }

    /// The operator's executable search space. Confinement grants these
    /// directories reads: executing a binary requires reading it, and an
    /// operator-selected script runtime resolves its real interpreter here.
    pub fn search_directories(&self) -> &[PathBuf] {
        &self.search_path
    }

    pub(crate) fn resolve(&self, program: &Path) -> io::Result<PathBuf> {
        if program.is_absolute() {
            return std::fs::canonicalize(program);
        }
        if program.components().count() != 1 || program.file_name() != Some(program.as_os_str()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a native executable must be absolute or a single operator-PATH name",
            ));
        }
        for directory in &self.search_path {
            let path = directory.join(program);
            match std::fs::metadata(&path) {
                Ok(metadata) => {
                    use std::os::unix::fs::PermissionsExt;
                    if metadata.is_file() && metadata.permissions().mode() & 0o111 != 0 {
                        return std::fs::canonicalize(path);
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                    ) => {}
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "executable is absent from operator PATH",
        ))
    }

    pub(crate) fn entries(
        &self,
        role: ProcessRole,
    ) -> impl Iterator<Item = (&OsStr, &OsStr)> + Clone {
        self.values.iter().filter_map(move |(key, value)| {
            (role == ProcessRole::Evaluator || key != OsStr::new("DENO_DIR"))
                .then_some((key.as_os_str(), value.as_os_str()))
        })
    }
}

fn operator_key(name: &str) -> bool {
    matches!(
        name,
        "HOME"
            | "USER"
            | "LOGNAME"
            | "PATH"
            | "LANG"
            | "TZ"
            | "TMPDIR"
            | "TMP"
            | "TEMP"
            | "DENO_DIR"
            | "XDG_CONFIG_HOME"
            | "XDG_DATA_HOME"
            | "XDG_CACHE_HOME"
            | "XDG_STATE_HOME"
            | "XDG_RUNTIME_DIR"
            | "DBUS_SESSION_BUS_ADDRESS"
            | "HTTP_PROXY"
            | "HTTPS_PROXY"
            | "ALL_PROXY"
            | "NO_PROXY"
            | "http_proxy"
            | "https_proxy"
            | "all_proxy"
            | "no_proxy"
            | "SSL_CERT_FILE"
            | "SSL_CERT_DIR"
    ) || (name.starts_with("LC_")
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'))
}
