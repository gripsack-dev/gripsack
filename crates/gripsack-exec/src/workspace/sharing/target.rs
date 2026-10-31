//! Portable v1 host requirements are declarative floors, never loader settings.
//! Admission measures the current machine; solve facts and CONDA_OVERRIDE_*
//! cannot authorize execution. Linux ABI bundles its userspace (or is static),
//! whereas glibc ABI additionally requires the measured host libc floor.
use super::format::failure;
use crate::ExecError;
use gripsack_conda::virtuals::{evaluate_system_requirements, measured_architecture};
use gripsack_ir::workspace::{PlatformArch, PlatformOs, WorkspacePlatform};
use gripsack_ir::workspace_model::lock::{
    LockedCondaSystemRequirements, LockedVirtualPackage, LockedVirtualPackageRequirement,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortableTarget {
    pub os: String,
    pub arch: String,
    pub abi: RuntimeAbi,
    pub minimum_kernel: String,
    pub cpu: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RuntimeAbi {
    Linux {},
    Glibc { minimum: String },
}

impl PortableTarget {
    /// Validate bounded numeric floors and a known x86_64 archspec CPU target.
    pub fn validate(&self) -> Result<(), ExecError> {
        if self.os != "linux" || self.arch != "x86_64" {
            return Err(failure("portable v1 requires target linux/x86_64"));
        }
        numeric_version(&self.minimum_kernel)?;
        if let RuntimeAbi::Glibc { minimum } = &self.abi {
            numeric_version(minimum)?;
        }
        if self.cpu.is_empty()
            || self.cpu.len() > 64
            || !self
                .cpu
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err(failure(
                "portable CPU must be a bounded archspec identifier",
            ));
        }
        // The existing archspec evaluator rejects unknown CPU names and proves
        // this CPU belongs to the selected x86_64 family, not merely any ISA.
        let cpu = LockedVirtualPackage {
            name: "__archspec".into(),
            version: "1".into(),
            build: self.cpu.clone(),
        };
        evaluate_system_requirements(
            &LockedCondaSystemRequirements {
                virtual_packages: Vec::new(),
                archspec: Some("x86_64".into()),
            },
            std::slice::from_ref(&cpu),
        )
        .map_err(|error| failure(error.to_string()))?;
        // Parse the floors through the same rattler version implementation as
        // native runtime admission, without using host measurements to validate
        // an otherwise well-formed declaration.
        let requirements = self.requirements();
        let mut facts = Vec::with_capacity(3);
        for required in &requirements.virtual_packages {
            facts.push(LockedVirtualPackage {
                name: required.name.clone(),
                version: required.minimum_version.clone(),
                build: "0".into(),
            });
        }
        facts.push(cpu);
        evaluate_system_requirements(&requirements, &facts)
            .map_err(|error| failure(error.to_string()))
    }

    /// Check current physical capabilities before import or managed consumption.
    /// This grants no authority to change library paths or execute manifest code.
    pub fn admit(&self) -> Result<(), ExecError> {
        self.validate()?;
        self.admit_platform(std::env::consts::OS, std::env::consts::ARCH)?;
        let platform = WorkspacePlatform {
            os: PlatformOs::Linux,
            arch: PlatformArch::X86_64,
            abi: None,
            minimum_os: None,
        };
        let mut facts = super::super::conda::host_virtual_packages("package", &platform)?;
        // The solve adapter deliberately returns a portable CPU baseline.
        // Replace it with the physical CPU, never an override or historic lock.
        facts.retain(|fact| fact.name != "__archspec");
        facts.push(measured_architecture().map_err(|error| failure(error.to_string()))?);
        self.admit_facts(&facts)
    }

    fn admit_platform(&self, os: &str, arch: &str) -> Result<(), ExecError> {
        if self.os != os || self.arch != arch {
            return Err(failure(
                "portable target differs from the native host OS/architecture",
            ));
        }
        Ok(())
    }

    fn admit_facts(&self, facts: &[LockedVirtualPackage]) -> Result<(), ExecError> {
        evaluate_system_requirements(&self.requirements(), facts)
            .map_err(|error| failure(error.to_string()))
    }

    fn requirements(&self) -> LockedCondaSystemRequirements {
        let mut virtual_packages = Vec::with_capacity(2);
        if let RuntimeAbi::Glibc { minimum } = &self.abi {
            virtual_packages.push(LockedVirtualPackageRequirement {
                name: "__glibc".into(),
                minimum_version: minimum.clone(),
                build: None,
            });
        }
        virtual_packages.push(LockedVirtualPackageRequirement {
            name: "__linux".into(),
            minimum_version: self.minimum_kernel.clone(),
            build: None,
        });
        LockedCondaSystemRequirements {
            virtual_packages,
            archspec: Some(self.cpu.clone()),
        }
    }
}

fn numeric_version(value: &str) -> Result<(), ExecError> {
    if value.is_empty()
        || value.len() > 32
        || value.split('.').count() > 4
        || !value.split('.').all(|part| {
            !part.is_empty()
                && part.bytes().all(|byte| byte.is_ascii_digit())
                && part.parse::<u32>().is_ok()
        })
    {
        return Err(failure(
            "portable version must contain at most four bounded decimal components (32 bytes)",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> PortableTarget {
        PortableTarget {
            os: "linux".into(),
            arch: "x86_64".into(),
            abi: RuntimeAbi::Glibc {
                minimum: "2.28".into(),
            },
            minimum_kernel: "4.18".into(),
            cpu: "x86_64".into(),
        }
    }

    fn facts() -> Vec<LockedVirtualPackage> {
        [
            ("__linux", "5.15", "0"),
            ("__glibc", "2.35", "0"),
            ("__archspec", "1", "x86_64"),
        ]
        .into_iter()
        .map(|(name, version, build)| LockedVirtualPackage {
            name: name.into(),
            version: version.into(),
            build: build.into(),
        })
        .collect()
    }

    #[test]
    fn measured_floors_reject_newer_kernel_libc_and_cpu() {
        let target = target();
        assert!(target.validate().is_ok());
        assert!(target.admit_facts(&facts()).is_ok());
        let mut newer = target.clone();
        newer.minimum_kernel = "6.0".into();
        assert!(newer.admit_facts(&facts()).is_err());
        newer = target.clone();
        newer.abi = RuntimeAbi::Glibc {
            minimum: "2.36".into(),
        };
        assert!(newer.admit_facts(&facts()).is_err());
        newer = target;
        newer.cpu = "x86_64_v3".into();
        assert!(newer.validate().is_ok());
        assert!(newer.admit_facts(&facts()).is_err());
    }

    #[test]
    fn linux_abi_needs_no_host_libc_but_still_needs_kernel_and_cpu() {
        let mut target = target();
        let mut facts = facts();
        facts.retain(|fact| fact.name != "__glibc");
        assert!(target.admit_facts(&facts).is_err());
        target.abi = RuntimeAbi::Linux {};
        assert!(target.admit_facts(&facts).is_ok());
        for missing in ["__linux", "__archspec"] {
            let incomplete: Vec<_> = facts
                .iter()
                .filter(|fact| fact.name != missing)
                .cloned()
                .collect();
            assert!(target.admit_facts(&incomplete).is_err());
        }
    }

    #[test]
    fn wrong_platform_unknown_cpu_and_unbounded_versions_reject() {
        let mut target = target();
        assert!(target.admit_platform("linux", "x86_64").is_ok());
        assert!(target.admit_platform("macos", "x86_64").is_err());
        assert!(target.admit_platform("linux", "aarch64").is_err());
        for cpu in ["unknown_cpu", "aarch64", "x86_64/../", ""] {
            target.cpu = cpu.into();
            assert!(target.validate().is_err());
        }
        target.cpu = "x86_64".into();
        for version in [
            "",
            "2..28",
            ">=2.28",
            "2.28rc1",
            "1.2.3.4.5",
            "4294967296",
            "111111111111111111111111111111111",
        ] {
            target.minimum_kernel = version.into();
            assert!(target.validate().is_err());
            target.minimum_kernel = "4.18".into();
            target.abi = RuntimeAbi::Glibc {
                minimum: version.into(),
            };
            assert!(target.validate().is_err());
            target.abi = RuntimeAbi::Linux {};
        }
        target.os = "macos".into();
        assert!(target.validate().is_err());
        target.os = "linux".into();
        target.arch = "aarch64".into();
        assert!(target.validate().is_err());
    }

    #[test]
    fn strict_target_and_tagged_abi_wire_shape() {
        let encoded = serde_json::to_value(target()).unwrap();
        assert_eq!(
            encoded["abi"],
            serde_json::json!({"kind":"glibc","minimum":"2.28"})
        );
        assert_eq!(
            serde_json::from_value::<PortableTarget>(encoded.clone()).unwrap(),
            target()
        );
        for abi in [
            serde_json::json!({"kind":"musl"}),
            serde_json::json!({"kind":"glibc"}),
            serde_json::json!({"kind":"linux","minimum":"2.28"}),
            serde_json::json!({"kind":"glibc","minimum":"2.28","library_path":"/tmp"}),
            serde_json::json!("linux"),
        ] {
            let mut malformed = encoded.clone();
            malformed["abi"] = abi;
            assert!(serde_json::from_value::<PortableTarget>(malformed).is_err());
        }
        let mut unknown = encoded;
        unknown["library_path"] = serde_json::json!("/tmp");
        assert!(serde_json::from_value::<PortableTarget>(unknown).is_err());
    }
}
