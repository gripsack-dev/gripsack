//! Common package admission for environments, profiles and explicit task commands.
use super::{ExecError, HostTarget, Package, PackageLayoutV6, Span, admit_target, gate};
use gripsack_ir::{
    HostFacts,
    workspace_v6::{identity::PackageDigest, lock::BytecodePolicy},
};
use std::{cell::RefCell, collections::BTreeSet, path::Path, sync::Arc, time::Instant};

/// Admission authority belongs to one invocation's home and measured host.
/// Package receipts have already been matched against protected realization;
/// the cache records compatibility, not an independent source of byte trust.
pub(in crate::workspace) struct NativeContext<'a> {
    pub(super) target: HostTarget,
    home: &'a Path,
    admitted: RefCell<BTreeSet<PackageDigest>>,
    deadline: Instant,
    gnu_loader: RefCell<Option<Arc<gripsack_process::SelectedGnuLoader>>>,
}
impl<'a> NativeContext<'a> {
    pub fn new(facts: &HostFacts, home: &'a Path, deadline: Instant) -> Result<Self, ExecError> {
        let mut target = HostTarget::from_facts(facts)?;
        let native_os = match target.os {
            gripsack_policy::target::TargetOs::Linux => "linux",
            gripsack_policy::target::TargetOs::Macos => "macos",
        };
        if native_os != std::env::consts::OS || facts.arch != std::env::consts::ARCH {
            return Err(super::failure(
                "native consumer facts do not describe the executing core's OS/architecture",
            ));
        }
        target.requirement.minimum_os = Some(crate::facts::platform_release()?.floor);
        Ok(Self {
            target,
            home,
            admitted: RefCell::default(),
            deadline,
            gnu_loader: RefCell::default(),
        })
    }

    pub(super) fn gnu_loader(
        &self,
        path: &'static str,
        span: &Span,
    ) -> Result<Arc<gripsack_process::SelectedGnuLoader>, ExecError> {
        let mut selected = self.gnu_loader.borrow_mut();
        if let Some(loader) = selected.as_ref() {
            if loader.path() != Path::new(path) {
                return Err(gate(
                    span,
                    "GNU interpreter differs from the invocation's platform loader",
                ));
            }
            return Ok(Arc::clone(loader));
        }
        let loader = gripsack_process::SelectedGnuLoader::select(Path::new(path), self.deadline)
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::Unsupported | std::io::ErrorKind::InvalidData => {
                    gate(span, error.to_string())
                }
                _ => super::operational(error),
            })?;
        let loader = Arc::new(loader);
        *selected = Some(Arc::clone(&loader));
        Ok(loader)
    }

    pub(super) fn admit_package_closure(
        &self,
        package: &Package,
        span: &Span,
    ) -> Result<Option<BytecodePolicy>, ExecError> {
        let mut pending = vec![package];
        let mut visited = BTreeSet::new();
        let mut admitted = self.admitted.borrow_mut();
        let mut bytecode = None;
        while let Some(package) = pending.pop() {
            if !visited.insert(package.identity) {
                continue;
            }
            pending.extend(package.runtime.iter().map(std::sync::Arc::as_ref));
            if let Some(receipt) = &package.conda {
                bytecode = Some(receipt.closure.materializer.bytecode);
            }
            if admitted.contains(&package.identity) {
                continue;
            }
            admit_target(&package.target, &self.target, span, "package")?;
            match (&package.layout, &package.conda) {
                (PackageLayoutV6::Relocatable, None) => {}
                (PackageLayoutV6::PrefixMaterialized, Some(receipt)) => {
                    crate::workspace::conda::admit_native(
                        receipt,
                        self.home,
                        &package.target,
                        &package.producer.payload,
                    )
                    .map_err(|error| gate(span, error.to_string()))?;
                    super::platform::admit(&receipt.system, &self.target, span)?;
                }
                (PackageLayoutV6::PrefixMaterialized, None) => {
                    return Err(gate(
                        span,
                        "prefix-materialized package has no independently validated Conda receipt",
                    ));
                }
                (PackageLayoutV6::Relocatable, Some(_)) => {
                    return Err(gate(
                        span,
                        "a Conda final-prefix artifact cannot claim a relocatable layout",
                    ));
                }
                (PackageLayoutV6::FixedPrefix { prefix }, _) => {
                    return Err(gate(
                        span,
                        format!(
                            "fixed prefix {:?} has no native placement authority; use a materialized Conda prefix or a relocatable package",
                            prefix.as_str()
                        ),
                    ));
                }
            }
            admitted.insert(package.identity);
        }
        Ok(bytecode)
    }
}
