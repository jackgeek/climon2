"""Opt-in #56 native input probe, restricted to one newly owned test window."""

import argparse
import json
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile
import time


def quote(value):
    if any(ord(char) < 32 for char in value):
        raise ValueError("control-bearing AppleScript argument")
    return '"' + value.replace("\\", "\\\\").replace('"', '\\"') + '"'


def script(source):
    result = subprocess.run(
        ["osascript", "-"], input=source, text=True, capture_output=True,
        timeout=15,
    )
    if result.returncode:
        raise RuntimeError(f"native automation failed: {result.stderr.strip()}")
    return result.stdout.strip()


def identity_guard(window, terminal):
    return f"""
        set ownedWindow to window id {window}
        if (count of tabs of ownedWindow) is not 1 then error "Owned window gained another tab"
        set ownedTab to tab 1 of ownedWindow
        if tty of ownedTab is not {quote(terminal)} then error "Owned TTY changed"
    """


def guard(window, terminal, marker):
    return identity_guard(window, terminal) + f"""
        if custom title of ownedTab is not {quote(marker)} then error "Owned marker changed"
    """


def send_key(window, terminal, marker, key_code):
    if key_code not in (126, 83):
        raise RuntimeError("unexpected native test key")
    script(f"""
    tell application "Terminal"
        {guard(window, terminal, marker)}
        activate
        set frontmost of ownedWindow to true
    end tell
    tell application "System Events"
        tell process "Terminal"
            if (count of (every window whose name contains {quote(marker)})) is not 1 then error "Ambiguous owned AX window"
            set ownedAX to first window whose name contains {quote(marker)}
            repeat 20 times
                if frontmost is true and value of attribute "AXMain" of ownedAX is true then exit repeat
                delay 0.05
            end repeat
            if frontmost is not true then error "Terminal is not focused"
            if value of attribute "AXMain" of ownedAX is not true then error "Owned window is not main"
        end tell
    end tell
    tell application "Terminal"
        {guard(window, terminal, marker)}
    end tell
    tell application "System Events"
        tell process "Terminal"
            if frontmost is not true then error "Focus changed"
            if value of attribute "AXMain" of ownedAX is not true then error "Main window changed"
            key code {key_code}
        end tell
    end tell
    """)


def wait_file(path, result_path):
    deadline = time.monotonic() + 12
    while not path.exists():
        if result_path.exists():
            raise RuntimeError("receiver ended before the next native stage")
        if time.monotonic() >= deadline:
            raise TimeoutError("native receiver checkpoint timed out")
        time.sleep(0.05)
    return json.loads(path.read_text())


def run(directory):
    receiver = Path(__file__).with_name("receiver.py")
    command = shlex.join([
        sys.executable, str(receiver), "--report-dir", str(directory),
    ]) + ' --expected-tty "$(tty)"'
    created = script(f"""
    tell application "Terminal"
        set oldIDs to id of every window
        set AppleScript's text item delimiters to ","
        set oldText to oldIDs as text
        set ownedTab to do script {quote(command)}
        return oldText & "|" & tty of ownedTab
    end tell
    """)
    old_text, terminal = created.split("|")
    if not terminal.startswith("/dev/ttys") or not terminal[9:].isdigit():
        raise RuntimeError("created tab returned an unexpected TTY; no window action attempted")
    old_ids = {int(value) for value in old_text.split(",") if value}
    marker = "CLIMON2-56-INPUT-" + directory.name
    # Bind by the returned TTY, never by the front window or tab index alone.
    window = int(script(f"""
    tell application "Terminal"
        set matches to {{}}
        repeat with candidate in every window
            if (count of tabs of candidate) is 1 then
                if tty of tab 1 of candidate is {quote(terminal)} then set end of matches to id of candidate
            end if
        end repeat
        if (count of matches) is not 1 then error "New test TTY is ambiguous"
        return item 1 of matches
    end tell
    """))
    if window in old_ids:
        raise RuntimeError("test TTY belongs to a pre-existing window; no window action attempted")
    (directory / "ownership.json").write_text(json.dumps({
        "window_id": window, "tty": terminal, "marker": marker,
    }))
    result_path = directory / "result.json"
    try:
        first = wait_file(directory / "ready-0.json", result_path)
        if first["tty"] != terminal:
            raise RuntimeError("receiver checkpoint belongs to another TTY")
        script(f"""
        tell application "Terminal"
            {identity_guard(window, terminal)}
            set custom title of ownedTab to {quote(marker)}
        end tell
        """)
        metadata = script(f"""
        tell application "Terminal"
            {guard(window, terminal, marker)}
            return version & "|" & name of current settings of ownedTab
        end tell
        """)
        (directory / "host.json").write_text(json.dumps({"host": metadata}))
        for index in range(6):
            stage = wait_file(directory / f"ready-{index}.json", result_path)
            if stage["tty"] != terminal:
                raise RuntimeError("receiver checkpoint belongs to another TTY")
            time.sleep(0.15)
            send_key(window, terminal, marker, stage["key_code"])
        report = wait_file(result_path, directory / "unused-end-marker")
        report["host"] = metadata
        report["boundary"] = (
            "System Events key codes on one dedicated Terminal.app PTY; "
            "standalone mode handoff, not physical keyboard or daemon integration"
        )
        if report["status"] != "PASS":
            raise RuntimeError("native receiver failed; unexpected input contents not recorded")
        return report
    finally:
        # Allow the bounded receiver to restore modes and exit before closing.
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            busy = script(f"""
            tell application "Terminal"
                {identity_guard(window, terminal)}
                return busy of ownedTab
            end tell
            """)
            if busy == "false":
                break
            time.sleep(0.1)
        script(f"""
        tell application "Terminal"
            {identity_guard(window, terminal)}
            if busy of ownedTab is not false then error "Owned receiver still busy; window left open"
            close ownedWindow saving no
        end tell
        """)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--permission-confirmed", action="store_true", required=True)
    args = parser.parse_args()
    if not args.permission_confirmed:
        parser.error("explicit dedicated-window permission required")
    parent = Path(__file__).parent / "artifacts"
    parent.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="native-", dir=parent) as directory:
        path = Path(directory)
        try:
            report = run(path)
        except (RuntimeError, TimeoutError, subprocess.TimeoutExpired) as error:
            report = {"status": "FAIL", "error": str(error)}
            if (path / "result.json").exists():
                report["receiver"] = json.loads((path / "result.json").read_text())
            if (path / "host.json").exists():
                report.update(json.loads((path / "host.json").read_text()))
            if (path / "ownership.json").exists():
                report["ownership"] = json.loads((path / "ownership.json").read_text())
            print(json.dumps(report, indent=2))
            raise SystemExit(1) from error
        print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
