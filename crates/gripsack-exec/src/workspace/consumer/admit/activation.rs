//! Conda activation environment (#405): the export-only effect of a prefix's
//! `activate.d` scripts, evaluated hermetically from the validated installed
//! tree. It is applied like any other environment contribution, as a default
//! the declaring environment or task may override.
use super::{ExecError, Span, gate};
use crate::workspace::artifact::Package;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;

pub(in crate::workspace::consumer) fn entries(
    package: &Package,
    span: &Span,
) -> Result<Vec<(OsString, OsString)>, ExecError> {
    let mut merged: BTreeMap<String, String> = BTreeMap::new();
    let mut pending = vec![package];
    let mut visited = BTreeSet::new();
    while let Some(package) = pending.pop() {
        if !visited.insert(package.identity) {
            continue;
        }
        pending.extend(package.runtime.iter().map(std::sync::Arc::as_ref));
        let Some(receipt) = &package.conda else {
            continue;
        };
        let exported = crate::workspace::conda::activation_environment(receipt)
            .map_err(|detail| gate(span, detail))?;
        for (name, value) in exported {
            if let Some(previous) = merged.insert(name.clone(), value.clone())
                && previous != value
            {
                return Err(gate(
                    span,
                    format!("two Conda prefixes activate {name} with different values"),
                ));
            }
        }
    }
    Ok(merged
        .into_iter()
        .map(|(name, value)| (OsString::from(name), OsString::from(value)))
        .collect())
}
