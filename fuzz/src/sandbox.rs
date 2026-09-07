use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub static SERIAL: Mutex<()> = Mutex::new(());

pub(crate) struct Sandbox {
    cap: gripsack_fs::Dir,
    temp: tempfile::TempDir,
    old: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

impl Sandbox {
    pub fn new() -> Self {
        let temp = tempfile::Builder::new()
            .prefix("gripsack-case-")
            .tempdir_in("/tmp")
            .unwrap();
        let cap = gripsack_fs::open_or_create(temp.path()).unwrap();
        let mut old = Vec::new();
        for key in [
            "HOME",
            "GRIPSACK_HOME",
            "XDG_DATA_HOME",
            "XDG_CONFIG_HOME",
            "XDG_CACHE_HOME",
            "TMPDIR",
        ] {
            old.push((key, std::env::var_os(key)));
            // All harness entries are serialized, and none starts worker threads.
            unsafe { std::env::set_var(key, temp.path()) };
        }
        Self { cap, temp, old }
    }

    pub fn home(&self) -> &Path {
        self.temp.path()
    }
    pub fn cap(&self) -> &gripsack_fs::Dir {
        &self.cap
    }
    pub fn fixed(&self, relative: &'static str) -> PathBuf {
        self.home().join(relative)
    }

    pub fn write(&self, relative: &'static str, bytes: &[u8]) {
        gripsack_fs::atomic_write(&self.cap, Path::new(relative), bytes).unwrap();
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        for (key, value) in &self.old {
            unsafe {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }
}
