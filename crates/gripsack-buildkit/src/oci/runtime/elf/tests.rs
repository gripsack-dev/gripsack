use super::*;
use crate::{oci::{BlobDigest, FileEntry, runtime::MetadataIndex},plan::{ImageConfig,LinuxOs,Platform}};
use sha2::{Digest,Sha256};

fn layout(needed: &[&str], launch: bool) -> ExecutableMetadata {
    ExecutableMetadata {
        format:Some(ExecutableFormat::Elf),class:Some(WordClass::SixtyFour),
        arch:Some(ExecutableArch::X86_64),endianness:Some(Endianness::Little),
        object:Some(ObjectKind::SharedObject),has_entry_point:launch,
        needed_libraries:needed.iter().map(|name| (*name).into()).collect(),
        ..ExecutableMetadata::default()
    }
}
fn object(files: &mut BTreeMap<String,FileEntry>,metadata: &mut MetadataIndex,path: &str,layout: ExecutableMetadata) -> BlobDigest {
    let digest = BlobDigest::from_bytes(Sha256::digest(path.as_bytes()).into());
    metadata.insert(digest,Ok(Box::new(layout)));
    files.insert(path.into(),FileEntry { mode:0o755,uid:0,gid:0,kind:FileKind::File { size:0,digest } });
    digest
}

#[test]
fn dependency_rpath_inheritance_is_not_runpath_inheritance() {
    let mut files = BTreeMap::new();
    for path in ["/","/app","/app/lib","/app/private","/lib64"] {
        files.insert(path.into(),FileEntry { mode:0o755,uid:0,gid:0,kind:FileKind::Directory });
    }
    let mut metadata = MetadataIndex::new();
    let root = object(&mut files,&mut metadata,"/app/program",ExecutableMetadata {
        interpreter:Some(Interpreter::Loader("/lib64/loader".into())),
        rpaths:vec!["$ORIGIN/lib:${ORIGIN}/private".into()],
        ..layout(&["libexample.so"],true)
    });
    object(&mut files,&mut metadata,"/lib64/loader",layout(&[],true));
    object(&mut files,&mut metadata,"/app/lib/libexample.so",layout(&["libinner.so"],false));
    let inner = object(&mut files,&mut metadata,"/app/private/libinner.so",layout(&[],false));
    let platform = Platform { os:LinuxOs::Linux,architecture:Architecture::Amd64 };
    let config = ImageConfig { entrypoint:vec!["/app/program".into()],args:vec![],env:vec![],cwd:"/".into(),user:"0:0".into() };
    validate(&Namespace { files:&files,metadata:&metadata,platform,config:&config }).unwrap();
    let program = metadata.get_mut(&root).unwrap().as_mut().unwrap();
    program.runpaths = std::mem::take(&mut program.rpaths);
    assert!(matches!(validate(&Namespace { files:&files,metadata:&metadata,platform,config:&config }),Err(OciError::Runtime { path,.. }) if path == "/app/lib/libexample.so"));
    let program = metadata.get_mut(&root).unwrap().as_mut().unwrap();
    program.rpaths = std::mem::take(&mut program.runpaths);
    metadata.get_mut(&inner).unwrap().as_mut().unwrap().arch = Some(ExecutableArch::Aarch64);
    assert!(matches!(validate(&Namespace { files:&files,metadata:&metadata,platform,config:&config }),Err(OciError::Runtime { path,.. }) if path == "/app/private/libinner.so"));
}
