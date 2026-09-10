"""Deterministic composite journal cases, including NUL framing and CRLF.

Archive seeds are REAL archives (gzip/xz/zip/tar through Python's own
encoders with pinned metadata: mtime/uid/gid 0, fixed timestamps) so the
fuzz corpus exercises the production decoder, never a shadow format.
Every seed is under the 64 KiB download cap.
"""
import gzip
import io
import json
import lzma
import tarfile
import zipfile
from pathlib import Path


def build(root: Path) -> None:
    journal = root / "journal"
    journal.mkdir(parents=True, exist_ok=True)
    for kind, prior in [
        ("absent", {"kind": "absent"}),
        ("file", {"kind": "file", "hash": "../../outside", "mode": 384}),
        ("symlink", {"kind": "symlink", "target": "/outside"}),
    ]:
        # the legacy pre-0.40 shape (fail-closed quarantine coverage)
        # and the tagged 0.40 shape (admission + decision coverage),
        # including the collision class: a link target spelling the
        # old removal sentinel or a file-identity-looking string
        entries = [
            json.dumps({"dest": "/outside", "prior": prior, "after": "gripsack:removed"}, sort_keys=True).encode(),
            json.dumps({"v": 1, "dest": "/outside", "prior": prior, "after": {"kind": "removed"}}, sort_keys=True).encode(),
            json.dumps({"v": 1, "dest": "/outside", "prior": prior, "after": {"kind": "link", "target": "gripsack:removed"}}, sort_keys=True).encode(),
            json.dumps({"v": 1, "dest": "/outside", "prior": prior, "after": {"kind": "file", "identity": "ab" * 32}}, sort_keys=True).encode(),
            json.dumps({"v": 1, "dest": "/outside", "prior": prior, "after": {"kind": "link", "target": "ab" * 32}}, sort_keys=True).encode(),
        ]
        for entry in entries:
            for previous, target, op in [(None, 1, "apply"), (3, 1, "rollback"), (1, 2, "apply"), (2, 3, "rollback")]:
                marker = json.dumps({"previous_generation": previous, "target_generation": target, "op": op}, sort_keys=True).encode()
                # Whitespace changes fixture current selection without altering Entry semantics.
                for prefix in [b"", b" ", b"\n", b"\t"]:
                    name = f"{kind}-{previous}-{target}-{op}-{len(prefix)}-{prefix.hex()}-{entry[:8].hex()}.json"
                    (journal / name).write_bytes(marker + b"\0" + prefix + entry)
    merge = root / "merge"
    merge.mkdir(parents=True, exist_ok=True)
    (merge / "crlf.txt").write_bytes(b"foreign\r\n# >>> gripsack module=m sha=abcd >>>\r\npayload\r\n# <<< gripsack module=m <<<\r\n")
    (merge / "payload.txt").write_bytes(b"foreign\n\0echo '{{ literal }}'\n")
    archive_seeds(root / "archive")


def archive_seeds(out: Path) -> None:
    out.mkdir(parents=True, exist_ok=True)

    def info(name, mode=0o644, size=0, kind=tarfile.REGTYPE, link=""):
        entry = tarfile.TarInfo(name)
        entry.mode, entry.size, entry.type, entry.linkname = mode, size, kind, link
        entry.mtime = entry.uid = entry.gid = 0
        entry.uname = entry.gname = ""
        return entry

    def tar_bytes(entries, fmt=tarfile.GNU_FORMAT):
        buffer = io.BytesIO()
        with tarfile.open(fileobj=buffer, mode="w", format=fmt) as tf:
            for meta, *content in entries:
                tf.addfile(meta, io.BytesIO(content[0]) if content else None)
        return buffer.getvalue()

    def zip_bytes(entries):
        buffer = io.BytesIO()
        with zipfile.ZipFile(buffer, "w") as zf:
            for name, data, mode, method in entries:
                zi = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
                zi.compress_type = method
                zi.create_system = 3
                zi.external_attr = (mode & 0xFFFF) << 16
                zf.writestr(zi, data)
        return buffer.getvalue()

    safe = [
        (info("bin/tool", 0o755, 13), b"#!/bin/sh\nid\n"),
        (info("share/doc/readme.txt", 0o644, 6), b"hello\n"),
        (info("share/link", 0o777, 0, tarfile.SYMTYPE, "../bin/tool"),),
        (info("share", 0o755, 0, tarfile.DIRTYPE),),
    ]
    hostile = safe + [
        (info("../escape.txt", 0o644, 4), b"out\n"),
        (info("/abs.txt", 0o644, 4), b"abs\n"),
    ]
    # 129 chars: past the 100-char ustar name field, forcing a GNU
    # long-name (L) metadata entry through the bounded admission pass.
    long_name = "n/" * 60 + "leaf.txt"
    (out / "safe.tar.gz").write_bytes(gzip.compress(tar_bytes(safe), compresslevel=9, mtime=0))
    (out / "hostile.tar.gz").write_bytes(gzip.compress(tar_bytes(hostile), compresslevel=9, mtime=0))
    (out / "safe.tar").write_bytes(tar_bytes(safe, tarfile.USTAR_FORMAT))
    (out / "safe.tar.xz").write_bytes(lzma.compress(tar_bytes(safe), format=lzma.FORMAT_XZ, preset=6))
    (out / "longname.tar").write_bytes(
        tar_bytes([(info(long_name, 0o644, 4), b"deep"), (info("hostile-" + "../" * 30 + "escape", 0o644, 4), b"bad\n")])
    )
    (out / "many.zip").write_bytes(zip_bytes([(f"e/{i:03d}.txt", b"x", 0o644, zipfile.ZIP_STORED) for i in range(200)]))
    (out / "safe.zip").write_bytes(
        zip_bytes([("bin/tool", b"#!/bin/sh\nid\n", 0o755, zipfile.ZIP_DEFLATED), ("empty/", b"", 0o755, zipfile.ZIP_STORED)])
    )
    # Decoder amplification under the 1 MiB expanded cap: a ~2 KB seed.
    (out / "bomb.zip").write_bytes(zip_bytes([("zeros", b"\0" * (4 * 1024 * 1024), 0o644, zipfile.ZIP_DEFLATED)]))
    (out / "bare.gz").write_bytes(gzip.compress(b"#!/bin/sh\necho hi\n", compresslevel=9, mtime=0))
    # Truncated magics and torn headers: parse-failure paths, no valid archive.
    (out / "truncated-pk.bin").write_bytes(b"PK\x03\x04" + b"\xde\xad\xbe\xef" * 8)
    (out / "truncated-gz.bin").write_bytes(b"\x1f\x8b" + b"\x99" * 24)
    (out / "truncated-xz.bin").write_bytes(b"\xfd7zXZ\x00" + b"\x00" * 16)
    (out / "ustar-garbage.bin").write_bytes(b"\0" * 257 + b"ustar" + b"\xff" * 200)
