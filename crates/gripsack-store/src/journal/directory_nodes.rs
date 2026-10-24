//! Each owned node has a permanent exclusive staging name outside the logical
//! tree. Retiring a node never deletes its identity while history references it.
use super::directory::{DirectoryIdentity, DirectoryTransition, invalid};
use gripsack_fs::Dir;
use std::{
    io,
    path::{Path, PathBuf},
};

pub(super) fn prepare(
    parent: &Dir,
    destination: &Path,
    relative: &Path,
) -> io::Result<(PathBuf, DirectoryIdentity)> {
    let name = format!(
        ".gripsack-directory-{}-{}",
        std::process::id(),
        crate::hash::hex_sha256(
            format!(
                "{destination:?}-{relative:?}-{:?}",
                std::time::SystemTime::now()
            )
            .as_bytes()
        )
    );
    let name = Path::new(&name);
    gripsack_fs::fault::operation(gripsack_fs::fault::Boundary::Mkdir, name, || {
        parent.create_dir(name)
    })?;
    let identity = DirectoryIdentity::observe(parent, name)?;
    gripsack_fs::fsync_dir(parent, name)?;
    gripsack_fs::fsync_dir(parent, Path::new("."))?;
    Ok((destination.with_file_name(name), identity))
}

pub(super) fn stage<'a>(receipt: &'a DirectoryTransition, relative: &Path) -> io::Result<&'a Path> {
    if relative.as_os_str().is_empty() {
        return Ok(&receipt.staging);
    }
    receipt
        .directory_staging
        .get(relative)
        .map(PathBuf::as_path)
        .ok_or_else(|| invalid("directory node has no retained staging name"))
}

pub(super) fn empty(directory: &Dir, name: &Path, identity: DirectoryIdentity) -> io::Result<()> {
    identity.verify(directory, name)?;
    let node = gripsack_fs::open_dir_nofollow(directory, name)?;
    match node.read_dir(".")?.next() {
        None => Ok(()),
        Some(Err(error)) => Err(error),
        Some(Ok(_)) => Err(io::Error::new(
            io::ErrorKind::DirectoryNotEmpty,
            "foreign or remaining child preserves owned directory",
        )),
    }
}

fn logical_parent(receipt: &DirectoryTransition, relative: &Path) -> io::Result<(Dir, PathBuf)> {
    if relative.as_os_str().is_empty() {
        return Ok((
            receipt.parent()?,
            PathBuf::from(
                receipt
                    .destination
                    .file_name()
                    .ok_or_else(|| invalid("missing root name"))?,
            ),
        ));
    }
    receipt.leaf_parent(&receipt.destination.join(relative))
}

enum Installation {
    Target,
    Original,
}

pub(super) fn install(receipt: &DirectoryTransition, relative: &Path) -> io::Result<()> {
    install_as(receipt, relative, Installation::Target)
}

pub(super) fn restore(receipt: &DirectoryTransition, relative: &Path) -> io::Result<()> {
    install_as(receipt, relative, Installation::Original)
}

fn install_as(
    receipt: &DirectoryTransition,
    relative: &Path,
    role: Installation,
) -> io::Result<()> {
    let identity = *receipt
        .directories
        .get(relative)
        .ok_or_else(|| invalid("missing owned node identity"))?;
    let outer = receipt.parent()?;
    let staged = stage(receipt, relative)?
        .file_name()
        .ok_or_else(|| invalid("missing staging name"))?;
    let (parent, name) = logical_parent(receipt, relative)?;
    match parent.symlink_metadata(&name) {
        Ok(_) => {
            identity.verify(&parent, &name)?;
            match outer.symlink_metadata(staged) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
                Ok(_) => {
                    return Err(invalid(
                        "staging name occupied while directory is installed",
                    ));
                }
            }
            return gripsack_fs::fsync_dir(&parent, Path::new("."));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    match role {
        Installation::Target => empty(&outer, Path::new(staged), identity)?,
        Installation::Original => identity.verify(&outer, Path::new(staged))?,
    }
    gripsack_fs::rename_noreplace_between(&outer, Path::new(staged), &parent, &name)?;
    gripsack_fs::fsync_dir(&outer, Path::new("."))?;
    gripsack_fs::fsync_dir(&parent, Path::new("."))
}

pub(super) fn retire(receipt: &DirectoryTransition, relative: &Path) -> io::Result<()> {
    let identity = *receipt
        .directories
        .get(relative)
        .ok_or_else(|| invalid("missing owned node identity"))?;
    let outer = receipt.parent()?;
    let staged = stage(receipt, relative)?
        .file_name()
        .ok_or_else(|| invalid("missing staging name"))?;
    match outer.symlink_metadata(staged) {
        Ok(_) => {
            empty(&outer, Path::new(staged), identity)?;
            return gripsack_fs::fsync_dir(&outer, Path::new("."));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let (parent, name) = logical_parent(receipt, relative)?;
    empty(&parent, &name, identity)?;
    gripsack_fs::rename_noreplace_between(&parent, &name, &outer, Path::new(staged))?;
    gripsack_fs::fsync_dir(&outer, Path::new("."))?;
    gripsack_fs::fsync_dir(&parent, Path::new("."))?;
    empty(&outer, Path::new(staged), identity)
}

pub(super) fn ordered(receipt: &DirectoryTransition, reverse: bool) -> Vec<&PathBuf> {
    let mut paths: Vec<_> = receipt.directories.keys().collect();
    paths.sort_by_key(|path| path.components().count());
    if reverse {
        paths.reverse();
    }
    paths
}

pub(super) fn install_all(receipt: &DirectoryTransition) -> io::Result<()> {
    for relative in ordered(receipt, false) {
        install(receipt, relative)?;
    }
    Ok(())
}

pub(super) fn restore_all(receipt: &DirectoryTransition) -> io::Result<()> {
    for relative in ordered(receipt, false) {
        restore(receipt, relative)?;
    }
    Ok(())
}

pub(super) fn retire_all(receipt: &DirectoryTransition) -> io::Result<()> {
    for relative in ordered(receipt, true) {
        retire(receipt, relative)?;
    }
    Ok(())
}

pub(super) fn staged(receipt: &DirectoryTransition) -> io::Result<()> {
    let parent = receipt.parent()?;
    for (relative, identity) in &receipt.directories {
        let name = stage(receipt, relative)?
            .file_name()
            .ok_or_else(|| invalid("missing node staging name"))?;
        empty(&parent, Path::new(name), *identity)?;
    }
    Ok(())
}

pub(super) fn discard(receipt: &DirectoryTransition, relative: &Path) -> io::Result<()> {
    let parent = receipt.parent()?;
    let name = stage(receipt, relative)?
        .file_name()
        .ok_or_else(|| invalid("missing node staging name"))?;
    discard_in(&parent, Path::new(name), receipt.directories[relative])
}

pub(super) fn discard_in(parent: &Dir, name: &Path, identity: DirectoryIdentity) -> io::Result<()> {
    match parent.symlink_metadata(name) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
        Ok(_) => {}
    }
    empty(parent, name, identity)?;
    gripsack_fs::fault::operation(gripsack_fs::fault::Boundary::Unlink, name, || {
        parent.remove_dir(name)
    })?;
    gripsack_fs::fsync_dir(parent, Path::new("."))
}
