"""Offline PTY checks of the throwaway native-input receiver."""

import json
from concurrent.futures import ThreadPoolExecutor
import os
from pathlib import Path
import pty
import select
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

from vscode import configure, gui, inspect_ax_tree, run


class ReceiverTests(unittest.TestCase):
    def test_disposable_bash_profile_does_not_load_inherited_shell_history(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            configure(root, "CLIMON2-56-TEST", inspect_only=True)
            settings = json.loads((root / "profile" / "User" / "settings.json").read_text())
            profile = settings["terminal.integrated.profiles.osx"]["NativeInputFixture"]
            history = root / "synthetic-history"
            history.write_text("echo SYNTHETIC-HISTORY-56\n")
            environment = os.environ.copy()
            environment["HISTFILE"] = str(history)
            environment["BASH_ENV"] = ""
            environment.update(profile.get("env", {}))
            result = subprocess.run(
                [profile["path"], *profile["args"], "-c", "set -o history; history -r; history"],
                env=environment, capture_output=True, text=True, timeout=3, check=True,
            )
            self.assertNotIn("SYNTHETIC-HISTORY-56", result.stdout)

    def test_receiver_waits_for_start_then_reports_absent_or_partial_input(self):
        for payload in (b"", b"x", b"\x1b[A"):
            with self.subTest(payload_length=len(payload)), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                start = root / "start"
                master, slave = pty.openpty()
                process = None
                try:
                    process = subprocess.Popen(
                        [sys.executable, str(Path(__file__).with_name("receiver.py")),
                         "--expected-tty", os.ttyname(slave), "--report-dir", directory,
                         "--input-timeout-seconds", "1", "--start-file", str(start),
                         "--suite", "cursor-normal"],
                        stdin=slave, stdout=slave, stderr=subprocess.PIPE,
                    )
                    ready = root / "ready-0.json"
                    deadline = time.monotonic() + 3
                    while not ready.exists() and process.poll() is None:
                        if time.monotonic() >= deadline:
                            self.fail("Receiver did not announce readiness")
                        time.sleep(0.02)
                    self.assertTrue(
                        ready.exists(),
                        process.communicate(timeout=3)[1].decode() if process.poll() is not None else "",
                    )
                    time.sleep(1.2)
                    self.assertIsNone(process.poll(), "Input deadline ran before the start gate")
                    start.touch()
                    if payload:
                        os.write(master, payload)
                    process.communicate(timeout=3)
                    report = json.loads((root / "result.json").read_text())
                    matched = payload == b"\x1b[A"
                    self.assertEqual(process.returncode == 0, matched)
                    self.assertEqual(report["status"], "PASS" if matched else "FAIL")
                    self.assertEqual(report["stages"][0]["received_length"], len(payload))
                    self.assertEqual(report["stages"][0]["matched"], matched)
                    self.assertEqual(report["stages"][0]["matched_expected_hex"],
                                     "1b5b41" if matched else None)
                finally:
                    if process is not None:
                        if process.poll() is None:
                            process.terminate()
                        process.communicate(timeout=3)
                    os.close(master)
                    os.close(slave)

    def test_ax_query_keeps_owned_process_alive_until_post_query_confirmation(self):
        for query_error, native_accessibility in (
            (None, False), (None, True), (RuntimeError("AX query failed"), True),
        ):
            with self.subTest(query_error=query_error, native_accessibility=native_accessibility), \
                    tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                focused = root / "focused"
                focused.touch()
                inspected = root / "inspected"
                queried = threading.Event()

                def query(*args):
                    queried.set()
                    if query_error is not None:
                        raise query_error
                    return "focused|missing"

                with subprocess.Popen(["/bin/sleep", "30"]) as process:
                    with patch("vscode.subprocess.Popen", return_value=process), \
                            patch("vscode.wait_window"), \
                            patch("vscode.gui", return_value="1"), \
                            patch("vscode.inspect_ax_tree", side_effect=query), \
                            patch("vscode.enable_native_accessibility", create=True) as enable, \
                            ThreadPoolExecutor(max_workers=1) as executor:
                        future = executor.submit(
                            run, root, terminal_focus_file=focused, inspect_only=True,
                            inspection_complete_file=inspected,
                            native_accessibility=native_accessibility,
                        )
                        try:
                            if not queried.wait(timeout=2):
                                future.result(timeout=1)
                                self.fail("AX query was not reached")
                            time.sleep(0.3)
                            self.assertIsNone(process.poll(), "Runner closed the window after the query")
                            self.assertFalse(future.done(), "Runner skipped post-query confirmation")
                        finally:
                            inspected.touch()
                            if not queried.is_set() and process.poll() is None:
                                process.terminate()
                        if query_error is None:
                            self.assertEqual(future.result(timeout=3)["status"], "INSPECTED")
                        else:
                            with self.assertRaisesRegex(RuntimeError, "AX query failed"):
                                future.result(timeout=3)
                        self.assertIsNotNone(process.poll(), "Owned process was not cleaned up")
                        if native_accessibility:
                            enable.assert_called_once_with(process.pid)
                        else:
                            enable.assert_not_called()

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

    def test_collect_all_stages_retains_failure_and_checks_later_handoff(self):
        payloads = [b"\x1b[A", b"\x1bOA", b"\x1b[A", b"1", b"unexpected-input", b"1"]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            master, slave = pty.openpty()
            try:
                with subprocess.Popen(
                    [sys.executable, str(Path(__file__).with_name("receiver.py")),
                     "--expected-tty", os.ttyname(slave), "--report-dir", directory,
                     "--collect-all-stages"],
                    stdin=slave, stdout=slave, stderr=subprocess.PIPE,
                ) as process:
                    for index, payload in enumerate(payloads):
                        checkpoint = root / f"ready-{index}.json"
                        deadline = time.monotonic() + 3
                        while not checkpoint.exists() and process.poll() is None:
                            if time.monotonic() >= deadline:
                                self.fail("receiver did not announce the next diagnostic stage")
                            time.sleep(0.02)
                        self.assertTrue(
                            checkpoint.exists(),
                            process.stderr.read().decode() if process.poll() is not None else "",
                        )
                        os.write(master, payload)
                    _, errors = process.communicate(timeout=3)
                    self.assertNotEqual(process.returncode, 0, errors.decode())
                    text = (root / "result.json").read_text()
                    report = json.loads(text)
                    self.assertEqual(report["status"], "FAIL")
                    self.assertEqual(
                        [stage["matched"] for stage in report["stages"]],
                        [True, True, True, True, False, True],
                    )
                    self.assertIsNone(report["stages"][4]["matched_expected_hex"])
                    self.assertNotIn("unexpected-input", text)
                    self.assertNotIn(b"unexpected-input".hex(), text)
            finally:
                os.close(master)
                os.close(slave)

    def test_receiver_can_wait_for_manual_focus_longer_than_key_deadline(self):
        expected = [b"\x1b[A", b"\x1bOA", b"\x1b[A", b"1", b"\x1bOq", b"1"]
        with tempfile.TemporaryDirectory() as directory:
            master, slave = pty.openpty()
            try:
                with subprocess.Popen(
                    [sys.executable, str(Path(__file__).with_name("receiver.py")),
                     "--expected-tty", os.ttyname(slave), "--report-dir", directory,
                     "--suite", "keys", "--input-timeout-seconds", "12"],
                    stdin=slave, stdout=slave, stderr=subprocess.PIPE,
                ) as process:
                    checkpoint = Path(directory) / "ready-0.json"
                    deadline = time.monotonic() + 5
                    while not checkpoint.exists() and process.poll() is None:
                        if time.monotonic() >= deadline:
                            self.fail("receiver did not announce first key stage")
                        if select.select([master], [], [], 0.05)[0]:
                            os.read(master, 4096)
                    self.assertTrue(
                        checkpoint.exists(),
                        process.stderr.read() if process.poll() is not None else "",
                    )
                    time.sleep(8.5)
                    self.assertIsNone(process.poll(), "receiver exited during manual focus wait")
                    for index, payload in enumerate(expected):
                        if index:
                            ready = Path(directory) / f"ready-{index}.json"
                            deadline = time.monotonic() + 5
                            while not ready.exists() and process.poll() is None:
                                if time.monotonic() >= deadline:
                                    self.fail("receiver did not announce the next key stage")
                                if select.select([master], [], [], 0.05)[0]:
                                    os.read(master, 4096)
                        os.write(master, payload)
                    _, errors = process.communicate(timeout=5)
                    self.assertEqual(process.returncode, 0, errors.decode())
                    report = json.loads((Path(directory) / "result.json").read_text())
                    self.assertEqual(report["status"], "PASS")
                    self.assertEqual(len(report["stages"]), len(expected))
            finally:
                os.close(master)
                os.close(slave)

    def test_inspection_profile_opens_a_persistent_bash_prompt(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            configure(root, "CLIMON2-56-TEST", inspect_only=True)
            settings = json.loads((root / "profile" / "User" / "settings.json").read_text())
            profile = settings["terminal.integrated.profiles.osx"]["NativeInputFixture"]
            self.assertEqual(profile["path"], "/bin/bash")
            self.assertEqual(profile["args"], ["--noprofile", "--norc"])
            master, slave = pty.openpty()
            try:
                with subprocess.Popen(
                    [profile["path"], *profile["args"]],
                    stdin=slave, stdout=slave, stderr=slave,
                    env={**os.environ, **profile["env"]},
                ) as process:
                    time.sleep(0.2)
                    self.assertIsNone(process.poll(), "Bash prompt exited without input")
                    os.write(master, b"exit\n")
                    self.assertEqual(process.wait(timeout=5), 0)
            finally:
                os.close(master)
                os.close(slave)

    def test_native_input_profile_runs_receiver_from_bash_and_keeps_shell_open(self):
        expected = [
            b"\x1b[A", b"\x1bOA", b"\x1b[A", b"1", b"\x1bOq", b"1",
            b"\x1b[200~# PASTE-56-ASCII\x1b[201~",
            b"# PASTE-56-ASCII",
            "\x1b[200~# PASTE-56-\u754ce\u0301\n# SECOND-56\x1b[201~".encode(),
            "# PASTE-56-\u754ce\u0301\r\n# SECOND-56".encode(),
        ]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            configure(root, "CLIMON2-56-TEST", collect_all_stages=True)
            settings = json.loads((root / "profile" / "User" / "settings.json").read_text())
            profile = settings["terminal.integrated.profiles.osx"]["NativeInputFixture"]
            self.assertEqual(profile["path"], "/bin/bash")
            master, slave = pty.openpty()
            output = bytearray()
            process = None

            def wait_until(predicate, message):
                deadline = time.monotonic() + 5
                while not predicate():
                    if select.select([master], [], [], 0.05)[0]:
                        output.extend(os.read(master, 4096))
                    if process.poll() is not None or time.monotonic() >= deadline:
                        self.fail(message)

            try:
                process = subprocess.Popen(
                    [profile["path"], *profile["args"]],
                    stdin=slave, stdout=slave, stderr=slave,
                    env={**os.environ, **profile["env"]},
                )
                for index, payload in enumerate(expected):
                    ready = root / "reports" / f"ready-{index}.json"
                    wait_until(ready.exists, "Bash did not start the receiver on the PTY")
                    os.write(master, payload)

                result = root / "reports" / "result.json"
                wait_until(result.exists, "Bash-launched receiver did not finish")
                report = json.loads(result.read_text())
                self.assertEqual(report["status"], "PASS")
                self.assertIsNone(process.poll(), "Bash exited with the receiver")
                wait_until(
                    lambda: b"NativeInputFixture exited with status 0" in output,
                    "Bash did not report the receiver exit status",
                )
                os.write(master, b"exit\n")
                self.assertEqual(process.wait(timeout=5), 0)
            finally:
                if process is not None and process.poll() is None:
                    process.terminate()
                    process.wait(timeout=5)
                os.close(master)
                os.close(slave)

    def test_manual_focus_profile_allows_time_to_focus_before_native_input(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            configure(root, "CLIMON2-56-TEST", manual_focus=True)
            settings = json.loads((root / "profile" / "User" / "settings.json").read_text())
            profile = settings["terminal.integrated.profiles.osx"]["NativeInputFixture"]
            self.assertIn("--input-timeout-seconds 8", profile["args"][3])
            self.assertIn("--start-file", profile["args"][3])

    def test_automatic_run_releases_start_only_after_terminal_guard_passes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            gate_checks = []

            def guard(pid, marker, action, require_terminal=False):
                if require_terminal:
                    gate_checks.append((root / "input-start").exists())
                    raise RuntimeError("Synthetic terminal focus unavailable")
                return "1"

            def checkpoint(path, result_path):
                return {"stage": "cursor-normal", "key_code": 126}

            with subprocess.Popen(["/bin/sleep", "30"]) as process:
                with patch("vscode.subprocess.Popen", return_value=process), \
                        patch("vscode.wait_window"), \
                        patch("vscode.gui", side_effect=guard), \
                        patch("vscode.wait_file", side_effect=checkpoint), \
                        patch("vscode.enable_native_accessibility", create=True), \
                        patch("vscode.inspect_ax_tree", side_effect=[
                            "focused|missing", "focused|terminal",
                        ]) as focused:
                    with self.assertRaisesRegex(RuntimeError, "Synthetic terminal focus unavailable"):
                        run(root, native_accessibility=True)
                    self.assertEqual(focused.call_count, 2)
                self.assertEqual(gate_checks, [False])
                self.assertFalse((root / "input-start").exists())
                settings = json.loads((root / "profile/User/settings.json").read_text())
                command = settings["terminal.integrated.profiles.osx"]["NativeInputFixture"]["args"][3]
                self.assertIn("--start-file", command)
                self.assertIsNotNone(process.poll())

    def test_terminal_input_guard_does_not_raise_window_before_checking_focus(self):
        with patch("vscode.script", side_effect=lambda value: value):
            source = gui(1234, "CLIMON2-56-TEST", "key code 126", require_terminal=True)
        self.assertNotIn("set frontmost of ownedProcess to true", source)
        self.assertNotIn('perform action "AXRaise" of ownedWindow', source)
        self.assertIn(
            'if focusedElement is missing value then error "Owned terminal input is not focused"',
            source,
        )

    def test_automatic_run_never_releases_input_on_invalid_focus_or_timeout(self):
        for observation, exception, message in (
            ("focused|other", RuntimeError, "Owned terminal input is not focused"),
            ("focused|query-error:-25212", RuntimeError, "focus query failed"),
            ("focused|missing", TimeoutError, "focus did not become available"),
        ):
            with self.subTest(observation=observation), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                with subprocess.Popen(["/bin/sleep", "30"]) as process:
                    with patch("vscode.subprocess.Popen", return_value=process), \
                            patch("vscode.wait_window"), \
                            patch("vscode.gui", return_value="1") as actions, \
                            patch("vscode.wait_file", return_value={"key_code": 126}), \
                            patch("vscode.enable_native_accessibility"), \
                            patch("vscode.inspect_ax_tree", return_value=observation), \
                            patch("vscode.time.monotonic", side_effect=[0, 0, 0, 11]):
                        with self.assertRaisesRegex(exception, message):
                            run(root, native_accessibility=True)
                        self.assertFalse((root / "input-start").exists())
                        self.assertFalse(any(
                            call.args[2] == "key code 126" for call in actions.call_args_list
                        ))
                    self.assertIsNotNone(process.poll())

    def test_ax_inspection_reports_only_focused_element_classification(self):
        with patch("vscode.script", side_effect=lambda value: value):
            source = inspect_ax_tree(1234, "CLIMON2-56-TEST")
        self.assertIn('set focusedClassification to "missing"', source)
        self.assertIn('"focused|" & focusedClassification', source)
        self.assertNotIn("value of focusedElement", source)
        self.assertNotIn("entire contents", source)

    def test_ax_role_conversion_runs_in_system_events_context(self):
        with patch("vscode.script", side_effect=lambda value: value):
            source = inspect_ax_tree(1234, "CLIMON2-56-TEST")
        handler = source.split('tell application "System Events"', 1)[0]
        conversion = next(
            line for line in source.splitlines()
            if line.strip().startswith("set elementRole to") and "safeText" in line
        )
        probe = handler + f'''
        tell application "System Events"
            set elementRef to {{role:"AXGroup"}}
            {conversion}
            return elementRole
        end tell
        '''
        result = subprocess.run(
            ["osascript", "-"], input=probe, text=True, capture_output=True, timeout=5,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), "AXGroup")


if __name__ == "__main__":
    unittest.main()
