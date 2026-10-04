//! Byte-pinned optional builder inputs. No ambient Lima installation is used.
use super::super::WorkerError;
use gripsack_process::{OperatorEnvironment, Sha256Digest};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Seek},
    num::NonZeroU64,
    path::Path,
    time::Instant,
};

pub(super) struct Asset {
    pub name: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub bytes: u64,
}
// Independently measured from the actual upstream release bytes, 2026-10-01.
pub(super) const LIMA: Asset = Asset {
    name: "lima.tar.gz",
    url: "https://github.com/lima-vm/lima/releases/download/v2.0.3/lima-2.0.3-Darwin-arm64.tar.gz",
    sha256: "22aee997df59e4fd448041b2d1214e48bd8eaf705d2d48a4307d65c1b179dc97",
    bytes: 37_829_370,
};
// Ubuntu's dated-image redirect targets an HTTP-only S3 website endpoint.
// Use that same official archive bucket's TLS REST endpoint directly; redirects
// remain HTTPS-only and the independently measured byte pin remains unchanged.
pub(super) const GUEST: Asset = Asset {
    name: "ubuntu.img",
    url: "https://s3.us-east-1.amazonaws.com/cloud-images-archive.ubuntu.com/releases/noble/release-20251213/ubuntu-24.04-server-cloudimg-arm64.img",
    sha256: "a40713938d74aaec811f74cb1fa8bfcb535d22e26b2a0ca1cc90ad9db898feb9",
    bytes: 620_884_992,
};
pub(super) const BUILDKIT: Asset = Asset {
    name: "buildkit.tar.gz",
    url: "https://github.com/moby/buildkit/releases/download/v0.33.0/buildkit-v0.33.0.linux-arm64.tar.gz",
    sha256: "e5acfb5929f967fde3b925ddb39f79fd481a0e96774c641fab3a0e83950d7bfa",
    bytes: 85_923_030,
};
pub(super) const LIMACTL_SHA256: &str =
    "64532072c7c6653d70ed8753ae582dc60d3dd13b56b81d664a634810d6785a02";
const AGENT_SHA256: &str = "e779bfa324051e4540b348ebc7c6e09affa1049cc90d3a7555b0b14bc1cf398e";
pub(super) const DAEMON_SHA256: &str =
    "3710911d9419cc9d830848c65283cadab7edc9c580193f6775503449851b5217";
pub(super) const CLIENT_SHA256: &str =
    "9920a0784eedd173fd3735351949ae13fa0671ef9afe8607f4d76ceafa5bfa03";

pub(super) fn verify(
    mut file: File,
    digest: &str,
    size: u64,
    deadline: Instant,
) -> Result<File, WorkerError> {
    if !file.metadata()?.is_file() || file.metadata()?.len() != size {
        return Err(WorkerError::Corrupt(
            "pinned builder input kind/length changed",
        ));
    }
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        if Instant::now() >= deadline {
            return Err(WorkerError::Deadline);
        }
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    if Sha256Digest::from_bytes(hash.finalize().into()) != Sha256Digest::parse(digest)? {
        return Err(WorkerError::Corrupt("pinned builder input SHA-256 changed"));
    }
    file.rewind()?;
    Ok(file)
}

pub(super) fn provision(
    directory: &gripsack_fs::Dir,
    asset: &Asset,
    environment: &OperatorEnvironment,
    offline: bool,
    deadline: Instant,
    // Retain exclusive lifecycle authority through synchronous publication.
    _lock: &gripsack_fs::FlockGuard,
) -> Result<File, WorkerError> {
    let provision = || -> Result<File, WorkerError> {
        match gripsack_fs::open_file_nofollow(directory, Path::new(asset.name)) {
            Ok(file) => return verify(file.into_std(), asset.sha256, asset.bytes, deadline),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        if offline {
            return Err(WorkerError::OfflineInput(asset.url));
        }
        let bytes = NonZeroU64::new(asset.bytes)
            .ok_or(WorkerError::Corrupt("empty pinned builder input"))?;
        let fetch = gripsack_fetch::FetchContext::from_operator(
            gripsack_fetch::FetchLimits {
                download_bytes: bytes,
                ..gripsack_fetch::FetchLimits::default()
            },
            environment,
        );
        let download = fetch
            .download_tool(asset.url, asset.sha256, bytes, deadline)
            .map_err(|error| WorkerError::Effect(error.to_string()))?;
        // download_tool has already hashed the bounded spool. Do not rehash
        // hundreds of MiB; only the exact manifest length remains to be checked.
        let mut spool = download.file.into_file();
        if spool.metadata()?.len() != asset.bytes {
            return Err(WorkerError::Corrupt("pinned builder input length changed"));
        }
        if Instant::now() >= deadline {
            return Err(WorkerError::Deadline);
        }
        spool.rewind()?;
        gripsack_fs::atomic_copy_with_mode(directory, Path::new(asset.name), &mut spool, 0o400)?;
        spool.rewind()?;
        Ok(spool)
    };
    provision().map_err(|error| match error {
        WorkerError::OfflineInput(_) | WorkerError::Deadline => error,
        _ => WorkerError::Effect(format!(
            "pinned builder input {} from {} failed: {error}",
            asset.name, asset.url,
        )),
    })
}

pub(super) fn install_lima(
    directory: &gripsack_fs::Dir,
    archive: File,
    deadline: Instant,
) -> Result<(), WorkerError> {
    // Only these two release members are needed for the built-in VZ driver.
    // No archive links, permissions, arbitrary paths or bundled plugins survive.
    for (member, digest, bytes, mode) in [
        ("bin/limactl", LIMACTL_SHA256, 30_138_768, 0o500),
        (
            "share/lima/lima-guestagent.Linux-aarch64.gz",
            AGENT_SHA256,
            15_150_257,
            0o400,
        ),
    ] {
        let path = Path::new(member);
        match gripsack_fs::open_file_nofollow(directory, path) {
            Ok(file) => {
                verify(file.into_std(), digest, bytes, deadline)?;
                continue;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let mut source = archive.try_clone()?;
        source.rewind()?;
        let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(source));
        let mut found = false;
        for entry in archive.entries()? {
            if Instant::now() >= deadline {
                return Err(WorkerError::Deadline);
            }
            let mut entry = entry?;
            let entry_path = entry.path()?;
            if entry_path.strip_prefix(".").unwrap_or(&entry_path) != path {
                continue;
            }
            if !entry.header().entry_type().is_file() || entry.size() != bytes {
                return Err(WorkerError::Corrupt(
                    "Lima release member changed kind/size",
                ));
            }
            let mut spool = tempfile::tempfile()?;
            std::io::copy(&mut entry, &mut spool)?;
            spool.rewind()?;
            let mut spool = verify(spool, digest, bytes, deadline)?;
            gripsack_fs::create_dir_all(directory, path.parent().expect("release member parent"))?;
            gripsack_fs::atomic_copy_with_mode(directory, path, &mut spool, mode)?;
            found = true;
            break;
        }
        if !found {
            return Err(WorkerError::Corrupt("Lima release lacks a required helper"));
        }
    }
    Ok(())
}
