"""Opt-in #56 input probe in an owned VS Code instance and scratch profile."""

import argparse
import ctypes
import ctypes.util
import json
import os
from pathlib import Path
import plistlib
import shlex
import shutil
import subprocess
import sys
import tempfile
import time

from terminal_app import quote, script, wait_file


EXECUTABLE = "/Applications/Visual Studio Code.app/Contents/MacOS/Code"


def enable_native_accessibility(pid):
    ax = ctypes.CDLL(ctypes.util.find_library("ApplicationServices"))
    cf = ctypes.CDLL(ctypes.util.find_library("CoreFoundation"))
    pointer = ctypes.c_void_p
    cf.CFStringCreateWithCString.argtypes = [pointer, ctypes.c_char_p, ctypes.c_uint32]
    cf.CFStringCreateWithCString.restype = pointer
    cf.CFRelease.argtypes = [pointer]
    ax.AXUIElementCreateApplication.argtypes = [ctypes.c_int]
    ax.AXUIElementCreateApplication.restype = pointer
    ax.AXUIElementSetAttributeValue.argtypes = [pointer, pointer, pointer]
    ax.AXUIElementSetAttributeValue.restype = ctypes.c_int
    application = ax.AXUIElementCreateApplication(pid)
    attribute = cf.CFStringCreateWithCString(None, b"AXManualAccessibility", 0x08000100)
    if not application or not attribute:
        if application:
            cf.CFRelease(application)
        if attribute:
            cf.CFRelease(attribute)
        raise RuntimeError("Could not create native accessibility references")
    try:
        error = ax.AXUIElementSetAttributeValue(
            application, attribute, pointer.in_dll(cf, "kCFBooleanTrue"),
        )
        if error != 0:
            raise RuntimeError(f"Owned native accessibility enablement failed: AX error {error}")
    finally:
        cf.CFRelease(attribute)
        cf.CFRelease(application)


def gui(pid, marker, action, require_terminal=False):
    activation = "" if require_terminal else """
        set frontmost of ownedProcess to true
        perform action "AXRaise" of ownedWindow
        repeat 20 times
            if frontmost of ownedProcess is true and value of attribute "AXMain" of ownedWindow is true then exit repeat
            delay 0.05
        end repeat
    """
    focus_guard = """
        set focusedElement to value of attribute "AXFocusedUIElement" of ownedProcess
        if focusedElement is missing value then error "Owned terminal input is not focused"
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
        {activation}
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


def inspect_ax_tree(pid, marker):
    return script(f"""
    on safeText(itemValue)
        if itemValue is missing value then return ""
        try
            return itemValue as text
        on error
            return "unavailable"
        end try
    end safeText
    tell application "System Events"
        set ownedProcesses to every application process whose unix id is {pid}
        if (count of ownedProcesses) is not 1 then error "Owned process missing or ambiguous"
        set ownedProcess to item 1 of ownedProcesses
        set ownedWindows to every window of ownedProcess whose name contains {quote(marker)}
        if (count of ownedWindows) is not 1 then error "Owned window missing or ambiguous"
        set ownedWindow to item 1 of ownedWindows
        set resultLines to {{"frontmost|" & (frontmost of ownedProcess as text)}}
        set focusedClassification to "missing"
        try
            set focusedElement to value of attribute "AXFocusedUIElement" of ownedProcess
            if focusedElement is not missing value then
                set focusedClassification to "other"
                set elementRef to focusedElement
                set elementRole to my safeText(role of elementRef)
                set elementSubrole to my safeText(subrole of elementRef)
                set elementFocused to my safeText(value of attribute "AXFocused" of elementRef)
                if (my safeText(description of focusedElement)) contains "Terminal" then
                    set focusedClassification to "terminal"
                end if
                set end of resultLines to elementRole & "|" & elementSubrole & "|" & elementFocused
            end if
        on error errorText number errorNumber
            set focusedClassification to "query-error:" & errorNumber
        end try
        set end of resultLines to "focused|" & focusedClassification
        set AppleScript's text item delimiters to linefeed
        return resultLines as text
    end tell
    """)


def configure(root, marker, inspect_only=False, manual_focus=False):
    profile = root / "profile" / "User"
    profile.mkdir(parents=True)
    workspace = root / marker
    workspace.mkdir()
    reports = root / "reports"
    reports.mkdir()
    if inspect_only:
        terminal_profile = {"path": "/bin/bash", "args": ["--noprofile", "--norc"]}
    else:
        receiver_command = shlex.join([
            sys.executable, str(Path(__file__).with_name("receiver.py")),
            "--expected-tty", "self", "--report-dir", str(reports),
            "--suite", "vscode", "--input-timeout-seconds",
            "600" if manual_focus else "8",
        ])
        shell_command = (
            f"{receiver_command}; receiver_status=$?; "
            'printf "\\nNativeInputFixture exited with status %s\\n" "$receiver_status"; '
            "exec /bin/bash --noprofile --norc"
        )
        terminal_profile = {
            "path": "/bin/bash",
            "args": ["--noprofile", "--norc", "-c", shell_command],
        }
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
            "NativeInputFixture": terminal_profile,
        },
    }
    (profile / "settings.json").write_text(json.dumps(settings, indent=2))
    return workspace, reports


def wait_for_human_file(path, process, timeout, message):
    deadline = time.monotonic() + timeout
    while not path.exists():
        if process.poll() is not None:
            raise RuntimeError(message)
        if time.monotonic() >= deadline:
            raise TimeoutError(message)
        time.sleep(0.2)


def run(root, window_ready_file=None, terminal_focus_file=None, inspect_only=False,
        inspection_complete_file=None, native_accessibility=False):
    marker = "CLIMON2-56-VSCODE-" + root.name
    workspace, reports = configure(
        root, marker, inspect_only, manual_focus=terminal_focus_file is not None,
    )
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
            if window_ready_file is not None:
                print(f"Owned VS Code window opened: {marker}. Waiting for manual readiness.", flush=True)
                wait_for_human_file(
                    window_ready_file, process, 600, "Manual window-readiness pause expired",
                )
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
            if native_accessibility:
                enable_native_accessibility(process.pid)
                print("Owned process accessibility requested through native AX API.", flush=True)
            if inspect_only:
                if terminal_focus_file is None:
                    raise RuntimeError("AX inspection requires manual focus confirmation")
                print("Inspect the Bash prompt, then focus the terminal pane and confirm readiness.", flush=True)
                wait_for_human_file(
                    terminal_focus_file, process, 600, "Manual focus confirmation expired",
                )
                try:
                    ax = inspect_ax_tree(process.pid, marker)
                    print("AX query returned; owned VS Code process alive: "
                          f"{process.poll() is None}", flush=True)
                    return {"status": "INSPECTED", "host": version,
                            "profile": "isolated NativeInputFixture; accessibility on, extensions disabled",
                            "native_accessibility_requested": native_accessibility,
                            "ax": ax,
                            "boundary": "Accessibility metadata only; no native key, paste or clipboard action"}
                finally:
                    if inspection_complete_file is not None:
                        print("AX query finished. Inspect the window before confirming cleanup.",
                              flush=True)
                        wait_for_human_file(
                            inspection_complete_file, process, 600,
                            "Post-query inspection expired or owned VS Code exited",
                        )
            result_path = reports / "result.json"
            if terminal_focus_file is not None:
                wait_file(reports / "ready-0.json", result_path)
                if process.poll() is not None or result_path.exists():
                    raise RuntimeError("Owned test process ended before manual focus")
                print("Inspect the Bash-backed fixture, focus its terminal pane, then confirm readiness.",
                      flush=True)
                wait_for_human_file(
                    terminal_focus_file, process, 600, "Manual focus confirmation expired",
                )
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
            report["native_accessibility_requested"] = native_accessibility
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
    parser.add_argument("--window-ready-file", type=Path)
    parser.add_argument("--terminal-focus-file", type=Path)
    parser.add_argument("--inspection-complete-file", type=Path)
    parser.add_argument("--inspect-only", action="store_true")
    parser.add_argument("--native-accessibility-permission-confirmed", action="store_true")
    args = parser.parse_args()
    if args.inspection_complete_file is not None and not args.inspect_only:
        parser.error("--inspection-complete-file requires --inspect-only")
    for path in (args.window_ready_file, args.terminal_focus_file, args.inspection_complete_file):
        if path is not None and path.exists():
            parser.error("manual readiness paths must be new")
    parent = Path(__file__).parent / "artifacts"
    parent.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="c56-", dir="/tmp") as directory:
        root = Path(directory)
        try:
            report = run(root, args.window_ready_file, args.terminal_focus_file, args.inspect_only,
                         args.inspection_complete_file, args.native_accessibility_permission_confirmed)
        except (RuntimeError, TimeoutError, subprocess.TimeoutExpired, subprocess.CalledProcessError) as error:
            report = {"status": "FAIL", "error": str(error),
                      "native_accessibility_requested": args.native_accessibility_permission_confirmed}
            report["announced_stages"] = [
                json.loads(path.read_text())["stage"]
                for path in sorted((root / "reports").glob("ready-*.json"))
            ]
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
