use super::*;
use std::io::Write;

fn extract_bytes(
    bytes: &[u8],
    out: &Path,
    name: &str,
    limits: FetchLimits,
) -> Result<(), FetchError> {
    let mut file = tempfile::tempfile().unwrap();
    file.write_all(bytes).unwrap();
    extract(&mut file, out, name, limits)
}

fn tar_entries(names: &[&str]) -> Vec<u8> {
    let mut builder = ::tar::Builder::new(Vec::new());
    for name in names {
        let mut header = ::tar::Header::new_gnu();
        header.set_size(1);
        header.set_mode(0o644);
        let bytes = name.as_bytes();
        header.as_old_mut().name[..bytes.len()].copy_from_slice(bytes);
        header.set_cksum();
        builder.append(&header, &b"x"[..]).unwrap();
    }
    builder.into_inner().unwrap()
}

#[test]
fn bare_and_compressed_binaries_preserve_executability() {
    let root = tempfile::tempdir().unwrap();
    let binary = b"\x7fELF fake binary";
    extract_bytes(binary, root.path(), "bare", FetchLimits::default()).unwrap();
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(binary).unwrap();
    extract_bytes(
        &encoder.finish().unwrap(),
        root.path(),
        "tool.gz",
        FetchLimits::default(),
    )
    .unwrap();
    assert_eq!(std::fs::read(root.path().join("tool")).unwrap(), binary);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(root.path().join("tool"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
    }
}

#[test]
fn xz_tar_extracts_without_a_payload_buffer() {
    let root = tempfile::tempdir().unwrap();
    let mut encoder = xz2::write::XzEncoder::new(Vec::new(), 6);
    encoder.write_all(&tar_entries(&["dir/file"])).unwrap();
    extract_bytes(
        &encoder.finish().unwrap(),
        root.path(),
        "archive.tar.xz",
        FetchLimits::default(),
    )
    .unwrap();
    assert_eq!(std::fs::read(root.path().join("dir/file")).unwrap(), b"x");
}

#[test]
fn traversal_is_rejected_before_tar_materialization() {
    let root = tempfile::tempdir().unwrap();
    let out = root.path().join("out");
    let result = extract_bytes(
        &tar_entries(&["ok", "../escape"]),
        &out,
        "archive.tar",
        FetchLimits::default(),
    );
    assert!(matches!(result, Err(FetchError::UnsafeArchive { .. })));
    assert!(!root.path().join("escape").exists());
    assert!(!out.join("ok").exists());
}

#[test]
fn decompressed_limit_is_checked_on_actual_bytes() {
    let root = tempfile::tempdir().unwrap();
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&vec![0; 256 * 1024]).unwrap();
    let limits = FetchLimits {
        expanded_bytes: std::num::NonZeroU64::new(1024).unwrap(),
        ..FetchLimits::default()
    };
    let result = extract_bytes(&encoder.finish().unwrap(), root.path(), "data.gz", limits);
    assert!(matches!(result, Err(FetchError::PayloadTooLarge { .. })));
    assert!(!root.path().join("data").exists());
}

fn zip_bytes(names: &[&str]) -> Vec<u8> {
    let mut writer = ::zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for name in names {
        writer
            .start_file(
                *name,
                ::zip::write::SimpleFileOptions::default()
                    .compression_method(::zip::CompressionMethod::Stored),
            )
            .unwrap();
        writer.write_all(b"payload").unwrap();
    }
    writer.finish().unwrap().into_inner()
}

#[test]
fn zip_entry_budget_precedes_directory_allocation() {
    let root = tempfile::tempdir().unwrap();
    let limits = FetchLimits {
        archive_entries: std::num::NonZeroUsize::new(1).unwrap(),
        ..FetchLimits::default()
    };
    let result = extract_bytes(
        &zip_bytes(&["one", "two"]),
        root.path(),
        "archive.zip",
        limits,
    );
    assert!(matches!(result, Err(FetchError::TooManyEntries { .. })));
    assert!(!root.path().join("one").exists());
}

#[test]
fn zip_crc_cannot_be_bypassed_by_raw_stream_decoding() {
    let root = tempfile::tempdir().unwrap();
    let mut bytes = zip_bytes(&["file"]);
    let central = bytes
        .windows(4)
        .position(|bytes| bytes == b"PK\x01\x02")
        .unwrap();
    bytes[central + 16..central + 20].fill(0);
    assert!(matches!(
        extract_bytes(&bytes, root.path(), "archive.zip", FetchLimits::default()),
        Err(FetchError::UnsafeArchive { .. })
    ));
}

#[test]
fn aggregate_tar_path_metadata_is_bounded_before_extraction() {
    let root = tempfile::tempdir().unwrap();
    let names: Vec<String> = (0..64)
        .map(|index| format!("directory/file-{index}"))
        .collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let limits = FetchLimits {
        decoder_bytes: std::num::NonZeroU64::new(8192).unwrap(),
        ..FetchLimits::default()
    };
    assert!(matches!(
        extract_bytes(&tar_entries(&names), root.path(), "archive.tar", limits),
        Err(FetchError::PayloadTooLarge { .. })
    ));
    assert!(!root.path().join("directory").exists());
}

#[cfg(unix)]
#[test]
fn a_symlink_destination_cannot_redirect_acquisition() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("payload"), b"contents").unwrap();
    let destination = root.path().join("out");
    std::os::unix::fs::symlink(outside.path(), &destination).unwrap();
    let spec = gripsack_ir::FetchSpec::File {
        path: root.path().join("payload").to_string_lossy().into_owned(),
    };
    assert!(
        crate::FetchContext::default()
            .fetch(&spec, &destination, None)
            .is_err()
    );
    assert!(!outside.path().join("payload").exists());
}
