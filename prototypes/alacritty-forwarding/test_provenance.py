"""Focused integrity checks for the throwaway dependency inventory."""

import hashlib
import io
from pathlib import Path
import tarfile
import tempfile
import unittest

from verify_provenance import verify_archive


class ProvenanceTests(unittest.TestCase):
    def test_archive_and_unpacked_source_must_both_match(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "fixture-1.0.0"
            root.mkdir()
            source = root / "lib.rs"
            source.write_bytes(b"original")
            archive = Path(directory) / "fixture.crate"
            with tarfile.open(archive, "w:gz") as output:
                member = tarfile.TarInfo("fixture-1.0.0/lib.rs")
                member.size = 8
                output.addfile(member, io.BytesIO(b"original"))
            checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
            self.assertEqual(verify_archive(root, archive, checksum), 1)
            source.write_bytes(b"modified")
            with self.assertRaisesRegex(ValueError, "Source differs"):
                verify_archive(root, archive, checksum)
            source.write_bytes(b"original")
            archive.write_bytes(archive.read_bytes() + b"altered")
            with self.assertRaisesRegex(ValueError, "Archive checksum differs"):
                verify_archive(root, archive, checksum)

    def test_archive_cannot_point_outside_its_package(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "fixture-1.0.0"
            root.mkdir()
            archive = Path(directory) / "fixture.crate"
            with tarfile.open(archive, "w:gz") as output:
                member = tarfile.TarInfo("fixture-1.0.0/../outside")
                member.size = 1
                output.addfile(member, io.BytesIO(b"x"))
            checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
            with self.assertRaisesRegex(ValueError, "Invalid archive path"):
                verify_archive(root, archive, checksum)


if __name__ == "__main__":
    unittest.main()
