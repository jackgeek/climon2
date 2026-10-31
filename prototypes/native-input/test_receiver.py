"""Offline PTY checks of the throwaway native-input receiver."""

import json
import os
from pathlib import Path
import pty
import select
import subprocess
import sys
import tempfile
import time
import unittest


class ReceiverTests(unittest.TestCase):
    def test_wrong_key_fails_without_recording_unexpected_contents(self):
        with tempfile.TemporaryDirectory() as directory:
            master, slave = pty.openpty()
            try:
                with subprocess.Popen(
                    [sys.executable, str(Path(__file__).with_name("receiver.py")),
                     "--expected-tty", os.ttyname(slave), "--report-dir", directory],
                    stdin=slave, stdout=slave, stderr=subprocess.PIPE,
                ) as process:
                    checkpoint = Path(directory) / "ready-0.json"
                    deadline = time.monotonic() + 5
                    while not checkpoint.exists():
                        if process.poll() is not None or time.monotonic() >= deadline:
                            self.fail("receiver did not announce its first stage")
                        if select.select([master], [], [], 0.05)[0]:
                            os.read(master, 4096)
                    os.write(master, b"unexpected-input")
                    process.communicate(timeout=5)
                    self.assertNotEqual(process.returncode, 0)
                    text = (Path(directory) / "result.json").read_text()
                    report = json.loads(text)
                    self.assertEqual(report["status"], "FAIL")
                    self.assertFalse(report["stages"][0]["matched"])
                    self.assertNotIn("unexpected-input", text)
            finally:
                os.close(master)
                os.close(slave)

    def test_mode_handoff_accepts_the_independent_expected_key_bytes(self):
        expected = [b"\x1b[A", b"\x1bOA", b"\x1b[A", b"1", b"\x1bOq", b"1"]
        with tempfile.TemporaryDirectory() as directory:
            (Path(directory) / "ownership.json").write_text('{"window_id": 1}')
            master, slave = pty.openpty()
            try:
                with subprocess.Popen(
                    [sys.executable, str(Path(__file__).with_name("receiver.py")),
                     "--expected-tty", os.ttyname(slave), "--report-dir", directory],
                    stdin=slave, stdout=slave, stderr=subprocess.PIPE,
                ) as process:
                    for index, payload in enumerate(expected):
                        deadline = time.monotonic() + 5
                        checkpoint = Path(directory) / f"ready-{index}.json"
                        while not checkpoint.exists() and process.poll() is None:
                            if time.monotonic() >= deadline:
                                self.fail("receiver did not announce the next stage")
                            if select.select([master], [], [], 0.05)[0]:
                                os.read(master, 4096)
                        self.assertTrue(checkpoint.exists(), process.stderr.read() if process.poll() is not None else "")
                        os.write(master, payload)
                    _, errors = process.communicate(timeout=5)
                    self.assertEqual(process.returncode, 0, errors.decode())
                    report = json.loads((Path(directory) / "result.json").read_text())
                    self.assertEqual(report["status"], "PASS")
                    self.assertEqual(len(report["stages"]), 6)
            finally:
                os.close(master)
                os.close(slave)


if __name__ == "__main__":
    unittest.main()
