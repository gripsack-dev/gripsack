//! Common package admission for environments, profiles and explicit task commands.
use super::{CatalogPackageLayout, ExecError, HostTarget, Package, Span, admit_target, gate};
use gripsack_ir::{
    HostFacts,
    workspace_model::{identity::PackageDigest, lock::BytecodePolicy},
};
use std::{cell::RefCell, collections::BTreeSet, path::Path, sync::Arc, time::Instant};

/// Admission authority belongs to one invocation's home and measured host.
/// Package receipts have already been matched against protected realization;
/// the cache records compatibility, not an independent source of byte trust.
pub(in crate::workspace) struct NativeContext<'a> {
    pub(super) target: HostTarget,
    pub(super) home: &'a Path,
    admitted: RefCell<BTreeSet<PackageDigest>>,
    imports: RefCell<crate::workspace::sharing::ReadmissionScope>,
    deadline: Instant,
    gnu_loader: RefCell<Option<Arc<gripsack_process::SelectedGnuLoader>>>,
}
impl<'a> NativeContext<'a> {
    pub fn new(facts: &HostFacts, home: &'a Path, deadline: Instant) -> Result<Self, ExecError> {
        let mut target = HostTarget::from_facts(facts)?;
        if std::env::consts::OS != "linux" || std::env::consts::ARCH != "x86_64" {
            return Err(super::failure(
                "native consumer facts do not describe the executing core's OS/architecture",
            ));
        }
        target.requirement.minimum_os = Some(crate::facts::platform_release()?.floor);
        Ok(Self {
            target,
            home,
            admitted: RefCell::default(),
            imports: RefCell::new(crate::workspace::sharing::ReadmissionScope::new(
                home,
                gripsack_fetch::FetchLimits::default(),
            )),
            deadline,
            gnu_loader: RefCell::default(),
        })
    }

    /// Receipt restoration and native admission share import evidence only for
    /// this invocation. Drop this borrow before entering native command checks.
    pub(in crate::workspace) fn readmission(
        &self,
        home: &Path,
    ) -> Result<std::cell::RefMut<'_, crate::workspace::sharing::ReadmissionScope>, ExecError> {
        if home != self.home {
            return Err(super::failure("native admission belongs to another home"));
        }
        Ok(self.imports.borrow_mut())
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
        let mut imports = self.imports.borrow_mut();
        while let Some(package) = pending.pop() {
            if !visited.insert(package.identity) {
                continue;
            }
            pending.extend(package.runtime.iter().map(std::sync::Arc::as_ref));
            imports
                .readmit_roots(
                    package
                        .producer
                        .retention
                        .iter()
                        .map(std::path::PathBuf::as_path),
                )
                .map_err(|error| gate(span, error.to_string()))?;
            if let Some(receipt) = &package.conda {
                bytecode = Some(receipt.closure.materializer.bytecode);
            }
            // Policy-dependent filesystem admission is never cached: ELF
            // command admission reconstructs the host roots for every launch.
            if package.host_runtime.is_some()
                && (package.target.os != gripsack_ir::workspace::PlatformOs::Linux
                    || package.target.abi != Some(gripsack_ir::workspace::PlatformAbi::Gnu)
                    || self.target.os != gripsack_policy::target::TargetOs::Linux
                    || self.target.abi != Some(super::BinaryAbi::Gnu))
            {
                return Err(gate(
                    span,
                    "host runtime directories require a native Linux GNU package",
                ));
            }
            if admitted.contains(&package.identity) {
                continue;
            }
            admit_target(&package.target, &self.target, span, "package")?;
            if package
                .producer
                .root
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with("-package-environment-projection"))
                && !matches!(package.layout, CatalogPackageLayout::PrefixMaterialized)
            {
                return Err(gate(
                    span,
                    "an imported environment projection must remain prefix-materialized",
                ));
            }
            match (&package.layout, &package.conda) {
                (CatalogPackageLayout::Relocatable, None) => {}
                (CatalogPackageLayout::PrefixMaterialized, Some(receipt)) => {
                    crate::workspace::conda::admit_native(
                        receipt,
                        self.home,
                        &package.target,
                        &package.producer.payload,
                    )
                    .map_err(|error| gate(span, error.to_string()))?;
                    super::platform::admit(&receipt.system, &self.target, span)?;
                }
                (CatalogPackageLayout::PrefixMaterialized, None) => {
                    crate::workspace::sharing::readmit_projection(
                        &mut imports,
                        &package.producer.payload,
                        &package.producer.retention,
                    )
                    .map_err(|error| gate(span, error.to_string()))?;
                }
                (CatalogPackageLayout::Relocatable, Some(_)) => {
                    return Err(gate(
                        span,
                        "a Conda final-prefix artifact cannot claim a relocatable layout",
                    ));
                }
                (CatalogPackageLayout::FixedPrefix { prefix }, _) => {
                    imports
                        .readmit_fixed(
                            &package.producer.payload,
                            Path::new(prefix.as_str()),
                            package
                                .producer
                                .retention
                                .iter()
                                .map(std::path::PathBuf::as_path),
                        )
                        .map_err(|error| gate(span, error.to_string()))?;
                }
            }
            admitted.insert(package.identity);
        }
        Ok(bytecode)
    }
}
