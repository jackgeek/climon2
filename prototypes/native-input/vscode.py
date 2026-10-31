"""Opt-in #56 input probe in an owned VS Code instance and scratch profile."""

import argparse
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys
import tempfile
import time

from terminal_app import quote, script, wait_file


EXECUTABLE = "/Applications/Visual Studio Code.app/Contents/MacOS/Code"


def gui(pid, marker, action, require_terminal=False):
    focus_guard = """
        set focusedElement to value of attribute "AXFocusedUIElement" of ownedProcess
        set focusedDescription to description of focusedElement
        if focusedDescription does not contain "Terminal" then error "Owned terminal input is not focused"
    """ if require_terminal else ""
    return script(f"""
    tell application "System Events"
        set ownedProcess to first application process whose unix id is {pid}
        set ownedWindows to every window of ownedProcess whose name contains {quote(marker)}
        if (count of ownedWindows) is not 1 then error "Owned VS Code window is ambiguous"
        set ownedWindow to item 1 of ownedWindows
        if exists sheet 1 of ownedWindow then error "Owned dialog pending; no approval sent"
        set frontmost of ownedProcess to true
        perform action "AXRaise" of ownedWindow
        repeat 20 times
            if frontmost of ownedProcess is true and value of attribute "AXMain" of ownedWindow is true then exit repeat
            delay 0.05
        end repeat
        if frontmost of ownedProcess is not true then error "Owned VS Code is not focused"
        if value of attribute "AXMain" of ownedWindow is not true then error "Owned window is not main"
        {focus_guard}
        {action}
        delay 0.25
        if exists sheet 1 of ownedWindow then error "Owned dialog appeared; no approval sent"
    end tell
    """)


def wait_window(process, marker):
    deadline = time.monotonic() + 25
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"Owned VS Code exited during startup: {process.returncode}")
        present = script(f"""
        tell application "System Events"
            set ownedProcesses to every application process whose unix id is {process.pid}
            if (count of ownedProcesses) is 0 then return false
            if (count of ownedProcesses) is not 1 then error "Owned process is ambiguous"
            set ownedProcess to item 1 of ownedProcesses
            return (count of (every window of ownedProcess whose name contains {quote(marker)})) is 1
        end tell
        """)
        if present == "true":
            return
        time.sleep(0.1)
    raise TimeoutError("Owned VS Code window did not appear")


def configure(root, marker):
    profile = root / "profile" / "User"
    profile.mkdir(parents=True)
    workspace = root / marker
    workspace.mkdir()
    reports = root / "reports"
    reports.mkdir()
    settings = {
        "workbench.startupEditor": "none",
        "window.restoreWindows": "none",
        "security.workspace.trust.enabled": False,
        "telemetry.telemetryLevel": "off",
        "update.mode": "none",
        "extensions.autoCheckUpdates": False,
        "extensions.autoUpdate": False,
        "editor.accessibilitySupport": "on",
        "terminal.integrated.enablePersistentSessions": False,
        "terminal.integrated.shellIntegration.enabled": False,
        "terminal.integrated.defaultProfile.osx": "NativeInputFixture",
        "terminal.integrated.profiles.osx": {
            "NativeInputFixture": {
                "path": sys.executable,
                "args": [str(Path(__file__).with_name("receiver.py")),
                         "--expected-tty", "self", "--report-dir", str(reports),
                         "--suite", "vscode"],
            },
        },
    }
    (profile / "settings.json").write_text(json.dumps(settings, indent=2))
    return workspace, reports


def run(root, human_ready_file=None):
    marker = "CLIMON2-56-VSCODE-" + root.name
    workspace, reports = configure(root, marker)
    with Path("/Applications/Visual Studio Code.app/Contents/Info.plist").open("rb") as source:
        version = plistlib.load(source)["CFBundleShortVersionString"]
    environment = os.environ.copy()
    environment.pop("VSCODE_IPC_HOOK_CLI", None)
    with (root / "launch.log").open("w") as launch_log:
        process = subprocess.Popen(
            [EXECUTABLE, "--new-window", "--disable-extensions",
             "--user-data-dir", str(root / "profile"),
             "--extensions-dir", str(root / "extensions"),
             "--shared-data-dir", str(root / "shared"),
             "--agents-user-data-dir", str(root / "agents-user"),
             "--agents-extensions-dir", str(root / "agents-extensions"),
             "--agent-plugins-dir", str(root / "agent-plugins"), str(workspace)],
            env=environment, stdout=launch_log, stderr=subprocess.STDOUT,
        )
        (root / "ownership.json").write_text(json.dumps({
            "pid": process.pid, "marker": marker,
        }))
        try:
            wait_window(process, marker)
            if human_ready_file is not None:
                print(f"Owned VS Code window opened: {marker}. Waiting for manual readiness.", flush=True)
                deadline = time.monotonic() + 600
                while not human_ready_file.exists():
                    if process.poll() is not None:
                        raise RuntimeError("Owned VS Code exited during manual readiness pause")
                    if time.monotonic() >= deadline:
                        raise TimeoutError("Manual readiness pause expired")
                    time.sleep(0.2)
            deadline = time.monotonic() + 20
            while gui(process.pid, marker, """
                tell ownedProcess
                    return count of (every menu bar item of menu bar 1 whose name is "Terminal")
                end tell
            """) != "1":
                if time.monotonic() >= deadline:
                    raise TimeoutError("Owned VS Code Terminal menu did not become ready")
                time.sleep(0.1)
            gui(process.pid, marker, """
                tell ownedProcess
                    click menu item "New Terminal" of menu 1 of menu bar item "Terminal" of menu bar 1
                end tell
            """)
            result_path = reports / "result.json"
            for index in range(10):
                stage = wait_file(reports / f"ready-{index}.json", result_path)
                if process.poll() is not None or result_path.exists():
                    raise RuntimeError("Owned test process ended before input")
                time.sleep(0.15)
                if "key_code" in stage:
                    key_code = stage["key_code"]
                    if key_code not in (126, 83):
                        raise RuntimeError("Unexpected native key")
                    action = f"key code {key_code}"
                else:
                    # Check focus before replacing the clipboard, then again at Paste.
                    gui(process.pid, marker, 'return "terminal-ready"', True)
                    subprocess.run(
                        ["pbcopy"], input=stage["paste_text"].encode("utf-8"),
                        check=True, timeout=5,
                    )
                    action = 'keystroke "v" using command down'
                gui(process.pid, marker, action, True)
            report = wait_file(result_path, root / "unused-end-marker")
            if report["status"] != "PASS":
                raise RuntimeError("Native input receiver failed")
            report["host"] = version
            report["profile"] = "isolated NativeInputFixture; accessibility on, extensions disabled"
            return report
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
                    raise RuntimeError("Owned VS Code required forced cleanup; normal cleanup not qualified")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--permission-confirmed", action="store_true", required=True)
    parser.add_argument("--clipboard-permission-confirmed", action="store_true", required=True)
    parser.add_argument("--human-ready-file", type=Path)
    args = parser.parse_args()
    if args.human_ready_file is not None and args.human_ready_file.exists():
        parser.error("manual readiness path must be new")
    parent = Path(__file__).parent / "artifacts"
    parent.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="c56-", dir="/tmp") as directory:
        root = Path(directory)
        try:
            report = run(root, args.human_ready_file)
        except (RuntimeError, TimeoutError, subprocess.TimeoutExpired, subprocess.CalledProcessError) as error:
            report = {"status": "FAIL", "error": str(error)}
            result = root / "reports" / "result.json"
            if result.exists():
                report["receiver"] = json.loads(result.read_text())
            if (root / "ownership.json").exists():
                report["ownership"] = json.loads((root / "ownership.json").read_text())
            if (root / "launch.log").exists():
                retained = parent / f"{root.name}-launch.log"
                shutil.copyfile(root / "launch.log", retained)
                report["launch_log_retained"] = retained.name
            print(json.dumps(report, indent=2))
            raise SystemExit(1) from error
        report["boundary"] = "Owned native Code instance; standalone mode handoff, not daemon integration"
        print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
