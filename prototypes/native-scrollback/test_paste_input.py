import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import termios
import unittest
from unittest.mock import Mock, patch

from paste_input import ModeUnavailable, query_mode, reported_mode
from terminal_paste import paste_stages
from test_interactive_input import wait_for


FIXTURE = Path(__file__).with_name("paste_input.py")
ATTEMPT = "0123456789abcdef"


class PasteTests(unittest.TestCase):
    def test_only_known_mutable_mode_replies_can_be_restored(self):
        self.assertIs(reported_mode(b"\x1b[?2004;1$y"), True)
        self.assertIs(reported_mode(b"\x1b[?2004;2$y"), False)
        for reply in [b"\x1b[?2004;0$y", b"\x1b[?2004;3$y", b"unrelated"]:
            with self.subTest(reply=reply), self.assertRaises(ModeUnavailable):
                reported_mode(reply)

    def run_fixture(self, mode, valid=True):
        with tempfile.TemporaryDirectory(prefix="climon2-paste-52-") as directory:
            report_file = Path(directory) / "report.json"
            master, slave = os.openpty()
            original = termios.tcgetattr(slave)
            child = subprocess.Popen(
                [sys.executable, str(FIXTURE), "--permission-confirmed",
                 "--expected-tty", os.ttyname(slave), "--attempt", ATTEMPT,
                 "--report-file", str(report_file)],
                stdin=slave, stdout=slave, stderr=subprocess.PIPE, start_new_session=True,
            )
            try:
                wait_for(master, b"\x1b[?2004$p")
                os.write(master, f"\x1b[?2004;{mode}$y".encode())
                if mode in [1, 2]:
                    wait_for(master, f"ON-52-{ATTEMPT}".encode())
                    data = b"PASTE-52-SYNTHETIC" if valid else b"UNEXPECTED"
                    os.write(master, b"\x1b[200~" + data + b"\x1b[201~")
                    if valid:
                        wait_for(master, f"OFF-52-{ATTEMPT}".encode())
                        os.write(master, b"PASTE-52-SYNTHETIC")
                        output = wait_for(master, f"OK-52-{ATTEMPT}".encode())
                    else:
                        output = b""
                else:
                    output = wait_for(master, f"NO-52-{ATTEMPT}".encode())
                child.communicate(timeout=5)
                restored = termios.tcgetattr(slave)
                original[3] &= ~termios.PENDIN
                restored[3] &= ~termios.PENDIN
                self.assertEqual(restored, original)
                report = json.loads(report_file.read_text()) if report_file.exists() else None
                return child.returncode, report, output
            finally:
                if child.poll() is None:
                    child.terminate()
                    try:
                        child.communicate(timeout=5)
                    except subprocess.TimeoutExpired:
                        child.kill()
                        child.communicate(timeout=5)
                child.stderr.close()
                os.close(master)
                os.close(slave)

    def test_pty_paste_framing_on_and_off_restores_both_original_states(self):
        for mode in [1, 2]:
            with self.subTest(mode=mode):
                code, report, output = self.run_fixture(mode)
                self.assertEqual(code, 0)
                self.assertTrue(report["bracketed_on_exact"])
                self.assertTrue(report["bracketed_off_exact"])
                self.assertTrue(report["input_modes_restored"])
                restore = b"\x1b[?2004h" if mode == 1 else b"\x1b[?2004l"
                self.assertIn(restore, output)

    def test_unknown_mode_stops_without_enabling_or_disabling_it(self):
        code, report, output = self.run_fixture(0)
        self.assertEqual(code, 2)
        self.assertEqual(report["status"], "blocked")
        self.assertTrue(report["input_modes_restored"])
        self.assertNotIn(b"\x1b[?2004h", output)
        self.assertNotIn(b"\x1b[?2004l", output)

    def test_unexpected_paste_fails_and_restores_without_recording_input(self):
        code, report, _ = self.run_fixture(2, valid=False)
        self.assertNotEqual(code, 0)
        self.assertIsNone(report)

    def test_query_timeout_is_an_explicit_unknown_state_not_a_guessed_mode(self):
        with patch("paste_input.emit"), patch("paste_input.read_bytes", side_effect=TimeoutError):
            with self.assertRaisesRegex(ModeUnavailable, "stopped before changing"):
                query_mode(0)

    def test_blocked_query_never_reaches_clipboard_or_native_paste(self):
        blocked = {"status": "blocked", "input_modes_restored": True}
        with patch("terminal_paste.wait_stage", return_value=blocked), \
                patch("terminal_paste.subprocess.run") as process, patch("terminal_paste.gui") as gui:
            self.assertEqual(paste_stages(258, "/dev/ttys008", ATTEMPT, Mock()), blocked)
            process.assert_not_called()
            gui.assert_not_called()


if __name__ == "__main__":
    unittest.main()
