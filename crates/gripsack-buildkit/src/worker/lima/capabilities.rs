//! Admission only: no helper selection, network access, or VM construction.
//! A positive result is not a boot/handshake qualification or a reservation of
//! host resources; VZ may still refuse a start if host conditions change.
use super::super::{WorkerError, WorkerOptions};
use objc2_foundation::NSProcessInfo;
use objc2_virtualization::{VZVirtualMachine, VZVirtualMachineConfiguration};
use std::time::Instant;

/// Call from acquire before preparing/downloading any optional builder inputs.
/// Status/open deliberately do not require virtualization-capable hardware.
pub(super) fn preflight(options: &WorkerOptions, deadline: Instant) -> Result<(), WorkerError> {
    if Instant::now() >= deadline {
        return Err(WorkerError::Deadline);
    }
    super::validate_options(options)?;
    let host = NSProcessInfo::processInfo();
    // Lima v2.0.3's VZ driver requires macOS 13 or newer.
    if host.operatingSystemVersion().majorVersion < 13 {
        return Err(WorkerError::Invalid(
            "the pinned Lima VZ builder requires macOS 13 or newer; upgrade macOS before requesting Linux builds",
        ));
    }
    // SAFETY: these are read-only class queries, available since macOS 11.
    // No virtual machine is constructed and no VM queue affinity is involved.
    // In particular, kern.hv_support is not an authority for VZ availability.
    let supported = unsafe { VZVirtualMachine::isSupported() };
    if !supported {
        return Err(WorkerError::Invalid(
            "Virtualization.framework reports VZVirtualMachine.isSupported=false; use an Apple Silicon Mac with VZ available (hosted VMs may lack nested virtualization); no builder inputs were downloaded",
        ));
    }
    let limits = unsafe {
        HostLimits {
            cpus: host.activeProcessorCount(),
            memory: host.physicalMemory(),
            minimum_cpus: VZVirtualMachineConfiguration::minimumAllowedCPUCount(),
            maximum_cpus: VZVirtualMachineConfiguration::maximumAllowedCPUCount(),
            minimum_memory: VZVirtualMachineConfiguration::minimumAllowedMemorySize(),
            maximum_memory: VZVirtualMachineConfiguration::maximumAllowedMemorySize(),
        }
    };
    limits.admit(usize::from(options.cpus.get()), options.memory.bytes())?;
    if Instant::now() >= deadline {
        return Err(WorkerError::Deadline);
    }
    Ok(())
}

/// Native boot identity for crash recovery, not a virtualization capability probe.
/// A query/shape failure is never replaced with a PID, time, or random identity.
pub(super) fn boot_session_id() -> Result<String, WorkerError> {
    let mut bytes = [0u8; 37];
    let mut length = bytes.len();
    // SAFETY: the fixed output buffer and its length are valid for the call;
    // null newp with zero newlen makes this a read-only sysctl.
    let result = unsafe {
        libc::sysctlbyname(
            c"kern.bootsessionuuid".as_ptr(),
            bytes.as_mut_ptr().cast(),
            &mut length,
            std::ptr::null_mut(),
            0,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    if length != bytes.len() || bytes[36] != 0 {
        return Err(WorkerError::Corrupt(
            "macOS boot session UUID has an invalid length",
        ));
    }
    for (index, byte) in bytes[..36].iter().enumerate() {
        if if matches!(index, 8 | 13 | 18 | 23) {
            *byte != b'-'
        } else {
            !byte.is_ascii_hexdigit()
        } {
            return Err(WorkerError::Corrupt(
                "macOS boot session UUID has an invalid shape",
            ));
        }
    }
    // Validation above proves ASCII. Normalize for stable persisted comparisons.
    Ok(std::str::from_utf8(&bytes[..36])
        .expect("validated ASCII UUID")
        .to_ascii_lowercase())
}

struct HostLimits {
    cpus: usize,
    memory: u64,
    minimum_cpus: usize,
    maximum_cpus: usize,
    minimum_memory: u64,
    maximum_memory: u64,
}
impl HostLimits {
    fn admit(&self, cpus: usize, memory: u64) -> Result<(), WorkerError> {
        if cpus < self.minimum_cpus || cpus > self.maximum_cpus || cpus > self.cpus {
            return Err(WorkerError::Invalid(
                "requested builder CPU count exceeds active host CPUs or Virtualization.framework limits; reduce the worker CPU limit",
            ));
        }
        if memory < self.minimum_memory || memory > self.maximum_memory || memory > self.memory {
            return Err(WorkerError::Invalid(
                "requested builder memory exceeds physical host memory or Virtualization.framework limits; choose a worker memory limit within the host and VZ bounds",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const GIB: u64 = 1024 * 1024 * 1024;
    fn limits() -> HostLimits {
        HostLimits {
            cpus: 8,
            memory: 16 * GIB,
            minimum_cpus: 1,
            maximum_cpus: 16,
            minimum_memory: GIB,
            maximum_memory: 32 * GIB,
        }
    }
    #[test]
    fn host_capacity_constrains_vz_ranges() {
        assert!(limits().admit(8, 16 * GIB).is_ok());
        assert!(matches!(
            limits().admit(9, 4 * GIB),
            Err(WorkerError::Invalid(_))
        ));
        assert!(matches!(
            limits().admit(2, 17 * GIB),
            Err(WorkerError::Invalid(_))
        ));
    }
    #[test]
    fn vz_ranges_constrain_host_capacity() {
        let limits = HostLimits {
            maximum_cpus: 4,
            maximum_memory: 8 * GIB,
            ..limits()
        };
        assert!(limits.admit(4, 8 * GIB).is_ok());
        assert!(matches!(
            limits.admit(5, 4 * GIB),
            Err(WorkerError::Invalid(_))
        ));
        assert!(matches!(
            limits.admit(2, 9 * GIB),
            Err(WorkerError::Invalid(_))
        ));
        assert!(matches!(
            limits.admit(0, 4 * GIB),
            Err(WorkerError::Invalid(_))
        ));
        assert!(matches!(
            limits.admit(2, GIB - 1),
            Err(WorkerError::Invalid(_))
        ));
    }
}
