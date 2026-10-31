"""Independent PTY examples for the native paste fixture."""

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


class PasteReceiverTests(unittest.TestCase):
    def test_paste_on_off_preserves_known_unicode_and_reports_newline_variant(self):
        unicode_cr = "# PASTE-56-\u754ce\u0301\r# SECOND-56".encode("utf-8")
        payloads = [
            b"\x1b[200~# PASTE-56-ASCII\x1b[201~",
            b"# PASTE-56-ASCII",
            b"\x1b[200~" + unicode_cr + b"\x1b[201~",
            unicode_cr.replace(b"\r", b"\r\n"),
        ]
        with tempfile.TemporaryDirectory() as directory:
            master, slave = pty.openpty()
            try:
                with subprocess.Popen(
                    [sys.executable, str(Path(__file__).with_name("receiver.py")),
                     "--suite", "paste", "--expected-tty", os.ttyname(slave),
                     "--report-dir", directory],
                    stdin=slave, stdout=slave, stderr=subprocess.PIPE,
                ) as process:
                    for index, payload in enumerate(payloads):
                        checkpoint = Path(directory) / f"ready-{index}.json"
                        deadline = time.monotonic() + 5
                        while not checkpoint.exists() and process.poll() is None:
                            if time.monotonic() >= deadline:
                                self.fail("paste receiver checkpoint timed out")
                            if select.select([master], [], [], 0.05)[0]:
                                os.read(master, 4096)
                        self.assertTrue(
                            checkpoint.exists(),
                            process.stderr.read() if process.poll() is not None else "",
                        )
                        os.write(master, payload)
                    _, errors = process.communicate(timeout=5)
                    self.assertEqual(process.returncode, 0, errors.decode())
                    report = json.loads((Path(directory) / "result.json").read_text())
                    self.assertEqual(report["status"], "PASS")
                    self.assertEqual(
                        [stage["matched_expected_hex"] for stage in report["stages"]],
                        [payload.hex() for payload in payloads],
                    )
            finally:
                os.close(master)
                os.close(slave)


if __name__ == "__main__":
    unittest.main()
