import json
import os
from pathlib import Path
import select
import subprocess
import sys
import tempfile
import time
import unittest


FIXTURE = Path(__file__).with_name("interactive_input.py")


def wait_for(master, marker):
    output = bytearray()
    deadline = time.monotonic() + 5
    while marker not in output:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise AssertionError(f"fixture did not emit {marker!r}")
        if select.select([master], [], [], remaining)[0]:
            chunk = os.read(master, 4096)
            if not chunk:
                raise AssertionError("unexpected fixture EOF")
            output.extend(chunk)
            if len(output) > 65536:
                raise AssertionError("fixture output exceeds its bound")
    return output


class InteractiveTests(unittest.TestCase):
    def run_fixture(self, text):
        with tempfile.TemporaryDirectory(prefix="climon2-native-input-52-") as directory:
            report = Path(directory) / "report.json"
            master, slave = os.openpty()
            child = subprocess.Popen(
                [sys.executable, str(FIXTURE), "--permission-confirmed",
                 "--expected-tty", os.ttyname(slave), "--report-file", str(report)],
                stdin=slave, stdout=slave, stderr=slave, start_new_session=True,
            )
            try:
                wait_for(master, b"INPUT-52-READY")
                os.write(master, text)
                child.wait(timeout=5)
                return child.returncode, json.loads(report.read_text()) if report.exists() else None
            finally:
                if child.poll() is None:
                    child.terminate()
                    child.wait(timeout=5)
                os.close(master)
                os.close(slave)

    def test_ordinary_pty_input_is_received_and_reported_without_raw_mode(self):
        code, report = self.run_fixture(b"INPUT-52-SYNTHETIC\n")
        self.assertEqual(code, 0)
        self.assertTrue(report["input_matched"])
        self.assertTrue(report["canonical_mode"])
        self.assertTrue(report["echo_enabled"])
        self.assertEqual(report["received"], "INPUT-52-SYNTHETIC")

    def test_unexpected_input_fails_without_recording_its_contents(self):
        code, report = self.run_fixture(b"not-the-synthetic-line\n")
        self.assertNotEqual(code, 0)
        self.assertIsNone(report)


if __name__ == "__main__":
    unittest.main()
