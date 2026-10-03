//! Repository build variables belong to child commands and artifact transport,
//! never to the grip process or trusted-tool provisioning context (0048 §1.2).

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::process::Command;

#[derive(Default)]
pub(crate) struct BuildProcessEnv {
    values: BTreeMap<String, String>,
    operator: Option<gripsack_process::OperatorEnvironment>,
}

impl BuildProcessEnv {
    pub(crate) fn new(values: BTreeMap<String, String>) -> Self {
        Self {
            values,
            operator: None,
        }
    }

    pub(crate) fn from_operator(environment: &gripsack_process::OperatorEnvironment) -> Self {
        Self {
            values: BTreeMap::new(),
            operator: Some(environment.clone()),
        }
    }

    pub(crate) fn apply(&self, command: &mut Command) {
        command.envs(&self.values);
    }

    pub(crate) fn var(&self, name: &str) -> Option<String> {
        if let Some(operator) = &self.operator {
            return operator
                .var_os(std::ffi::OsStr::new(name))
                .and_then(|value| value.to_str())
                .map(str::to_owned);
        }
        self.values
            .get(name)
            .cloned()
            .or_else(|| std::env::var(name).ok())
    }

    pub(crate) fn var_os(&self, name: &str) -> Option<OsString> {
        if let Some(operator) = &self.operator {
            return operator
                .var_os(std::ffi::OsStr::new(name))
                .map(OsString::from);
        }
        self.values
            .get(name)
            .map(OsString::from)
            .or_else(|| std::env::var_os(name))
    }
}
