#!/usr/bin/env python3
"""Actual filesystem publication boundaries; no signature-verification claim."""
import errno
import hashlib
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import release_consumer as consumer


class PublicationBoundary(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.parent = self.root / "bin"
        self.parent.mkdir()
        self.source = self.root / "candidate"
        self.source.write_bytes(b"new executable")
        self.destination = self.parent / "grip"
        self.destination.write_bytes(b"old executable")

    def test_directory_descriptor_failure_is_precommit(self):
        real = os.open

        def open_file(path, flags, *args, **kwargs):
            if os.fspath(path) == os.fspath(self.parent) and flags & os.O_DIRECTORY:
                raise PermissionError(errno.EACCES, "injected directory admission failure")
            return real(path, flags, *args, **kwargs)

        with patch.object(consumer.os, "open", side_effect=open_file):
            with self.assertRaises(PermissionError):
                consumer.publish(self.source, self.destination)
        self.assertEqual(self.destination.read_bytes(), b"old executable")
        self.assertEqual(list(self.parent.glob(".grip-install-*")), [])

    def test_directory_sync_failure_is_committed_not_rolled_back(self):
        real = os.fsync

        def sync(fd):
            if stat.S_ISDIR(os.fstat(fd).st_mode):
                raise OSError(errno.EIO, "injected directory sync failure")
            return real(fd)

        with patch.object(consumer.os, "fsync", side_effect=sync):
            with self.assertRaises(consumer.PublicationCommittedError):
                consumer.publish(self.source, self.destination)
        self.assertEqual(self.destination.read_bytes(), b"new executable")
        self.assertEqual(list(self.parent.glob(".grip-install-*")), [])

    def test_parent_replacement_never_redirects_the_commit(self):
        real = os.replace
        moved = self.root / "original-bin"

        def replace(source, destination, **kwargs):
            self.parent.rename(moved)
            self.parent.mkdir()
            (self.parent / "grip").write_bytes(b"foreign replacement")
            return real(source, destination, **kwargs)

        with patch.object(consumer.os, "replace", side_effect=replace):
            with self.assertRaises(consumer.PublicationCommittedError):
                consumer.publish(self.source, self.destination)
        self.assertEqual(self.destination.read_bytes(), b"foreign replacement")
        self.assertEqual((moved / "grip").read_bytes(), b"new executable")

    def test_symlink_and_fifo_lock_do_not_acquire_publication_authority(self):
        foreign = self.root / "foreign"
        foreign.write_bytes(b"preserved")
        lock = self.parent / f".grip-self-update-{hashlib.sha256(b'grip').hexdigest()}.flock"
        lock.symlink_to(foreign)
        with self.assertRaises(OSError):
            consumer.publish(self.source, self.destination)
        lock.unlink()
        os.mkfifo(lock)
        with self.assertRaises(ValueError):
            consumer.publish(self.source, self.destination)
        self.assertEqual(self.destination.read_bytes(), b"old executable")
        self.assertEqual(foreign.read_bytes(), b"preserved")


class VerifierProcessBoundary(unittest.TestCase):
    def test_combined_output_limit_rejects_excess_and_reaps_the_child(self):
        with self.assertRaises(ValueError):
            consumer.command([sys.executable, "-c", "import os; os.write(1, b'x' * 65536)"], maximum=1024)

    def test_deadline_is_not_success(self):
        with self.assertRaises((TimeoutError, subprocess.TimeoutExpired)):
            consumer.command([sys.executable, "-c", "import time; time.sleep(10)"], timeout=0.05)


if __name__ == "__main__":
    unittest.main()
