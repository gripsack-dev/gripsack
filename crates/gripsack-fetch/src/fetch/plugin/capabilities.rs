//! Optional, cached capability negotiation. Failed probes declare no budgets.

use super::protocol::PluginMessage;
use gripsack_process::{Control, Limits, StopReason};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

const CAPABILITIES_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Deserialize, Default, Clone)]
pub(super) struct Capabilities {
    #[serde(default)]
    pub throttle: BTreeMap<String, String>,
}

type Cache = BTreeMap<String, Option<Capabilities>>;

pub(super) fn get(
    context: &crate::FetchContext,
    name: &str,
    exe: &Path,
    deadline: std::time::Instant,
) -> Option<Capabilities> {
    static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(|| Mutex::new(BTreeMap::new()));
    if let Some(entry) = CACHE.lock().expect("caps cache").get(name) {
        return entry.clone();
    }
    let mut command = Command::new(exe);
    context.apply_build_env(&mut command);
    let caps = exchange(
        &mut command,
        Limits {
            timeout: CAPABILITIES_TIMEOUT,
            operation_deadline: Some(deadline),
            ..Limits::default()
        },
    );
    CACHE
        .lock()
        .expect("caps cache")
        .insert(name.to_owned(), caps.clone());
    caps
}

pub(super) fn exchange(command: &mut Command, limits: Limits) -> Option<Capabilities> {
    let mut caps = None;
    let outcome = gripsack_process::run(command, b"{\"op\":\"capabilities\"}\n", limits, |line| {
        let line = String::from_utf8_lossy(line);
        let Ok(message) = serde_json::from_str::<PluginMessage>(&line) else {
            return Control::Continue;
        };
        if message.kind != "response" {
            return Control::Continue;
        }
        caps = message
            .result
            .and_then(|r| r.capabilities)
            .and_then(|value| serde_json::from_value(value).ok());
        Control::Response
    })
    .ok()?;
    if matches!(outcome.reason, StopReason::Exited)
        && outcome.status.as_ref().is_some_and(|s| s.success())
    {
        caps
    } else {
        None
    }
}
