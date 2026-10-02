//! Repository build variables belong to child commands and artifact transport,
//! never to the grip process or trusted-tool provisioning context (0048 §1.2).

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::process::Command;

#[derive(Default)]
pub(crate) struct BuildProcessEnv {
    values: BTreeMap<String, String>,
}

impl BuildProcessEnv {
    pub(crate) fn new(values: BTreeMap<String, String>) -> Self {
        Self { values }
    }

    pub(crate) fn apply(&self, command: &mut Command) {
        command.envs(&self.values);
    }

    pub(crate) fn var(&self, name: &str) -> Option<String> {
        self.values
            .get(name)
            .cloned()
            .or_else(|| std::env::var(name).ok())
    }

    pub(crate) fn var_os(&self, name: &str) -> Option<OsString> {
        self.values
            .get(name)
            .map(OsString::from)
            .or_else(|| std::env::var_os(name))
    }
}
