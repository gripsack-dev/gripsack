//! Native Linux kernel capability, measured once without shell or repo overrides.
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
    #[cfg(not(target_os = "linux"))]
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
