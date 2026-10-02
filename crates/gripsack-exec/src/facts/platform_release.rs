//! Native OS capability, measured once without invoking a shell or reading a
//! repository override. Linux uses the kernel release; macOS uses the product
//! version rather than its unrelated Darwin ABI version.
use gripsack_policy::target::OsRelease;
use std::{ffi::CStr, io, sync::LazyLock};

pub(crate) struct PlatformRelease {
    pub version: String,
    pub floor: OsRelease,
}

pub(crate) fn platform_release() -> io::Result<&'static PlatformRelease> {
    static RELEASE: LazyLock<io::Result<PlatformRelease>> = LazyLock::new(measure);
    RELEASE
        .as_ref()
        .map_err(|error| io::Error::new(error.kind(), error.to_string()))
}

fn measure() -> io::Result<PlatformRelease> {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: uname initializes a correctly sized writable utsname; release
        // is a fixed NUL-terminated field on success.
        let mut uts: libc::utsname = unsafe { std::mem::zeroed() };
        if unsafe { libc::uname(&mut uts) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let release = unsafe { CStr::from_ptr(uts.release.as_ptr()) }
            .to_str()
            .map_err(io::Error::other)?;
        let end = release
            .find(|ch: char| !ch.is_ascii_digit() && ch != '.')
            .unwrap_or(release.len());
        parse(&release[..end])
    }
    #[cfg(target_os = "macos")]
    {
        let mut bytes = [0u8; 64];
        let mut length = bytes.len();
        // SAFETY: bounded output and size pointer remain valid for the query.
        if unsafe {
            libc::sysctlbyname(
                c"kern.osproductversion".as_ptr(),
                bytes.as_mut_ptr().cast(),
                &mut length,
                std::ptr::null_mut(),
                0,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        let bytes = bytes
            .get(..length)
            .ok_or_else(|| io::Error::other("macOS product version exceeds its response bound"))?;
        let version = CStr::from_bytes_until_nul(bytes)
            .map_err(io::Error::other)?
            .to_str()
            .map_err(io::Error::other)?;
        parse(version)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Err(io::Error::other(
            "native OS version measurement is unsupported on this platform",
        ))
    }
}

fn parse(version: &str) -> io::Result<PlatformRelease> {
    let mut components = version.split('.');
    let mut next = || -> io::Result<u16> {
        components
            .next()
            .ok_or_else(|| io::Error::other("missing native OS version component"))?
            .parse()
            .map_err(io::Error::other)
    };
    let major = next()?;
    let minor = next()?;
    let patch = components
        .next()
        .map(str::parse)
        .transpose()
        .map_err(io::Error::other)?
        .unwrap_or(0);
    // Linux may report additional numeric vendor components. They cannot make
    // the first three components less capable; reject malformed/overflow data.
    for component in components {
        let _: u32 = component.parse().map_err(io::Error::other)?;
    }
    Ok(PlatformRelease {
        version: version.into(),
        floor: OsRelease {
            major,
            minor,
            patch,
        },
    })
}
