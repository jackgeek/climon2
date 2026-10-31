"""Permissioned #52 Terminal.app GUI/clipboard oracle, not product integration."""

import argparse
import json
from pathlib import Path
import shlex
import subprocess
import sys
import time

from terminal_app import script


COPY_TARGETS = {
    "soft": "SOFT01234567890123456789tail",
    "hard": "HARD-A\n",
    "wide": "界e\u0301",
}


def quote(value):
    if any(ord(c) < 32 for c in value):
        raise ValueError("control-bearing AppleScript argument")
    return '"' + value.replace("\\", "\\\\").replace('"', '\\"') + '"'


def gui(window, tty, action):
    name = script(window, tty, f"name of window id {window}")
    if "CLIMON2-52-NATIVE-TEST" not in name:
        raise RuntimeError("dedicated GUI target title no longer matches")
    source = f"""
tell application "Terminal"
    set testTab to tab 1 of window id {window}
    if tty of testTab is not "{tty}" then error "Test tab identity changed"
    if (count of tabs of window id {window}) is not 1 then error "Dedicated window gained another tab"
    activate
    set frontmost of window id {window} to true
end tell
tell application "System Events"
    tell process "Terminal"
        set targetWindow to window {quote(name)}
        repeat 20 times
            if frontmost is true and value of attribute "AXMain" of targetWindow is true then exit repeat
            delay 0.05
        end repeat
        if frontmost is not true then error "Terminal is not focused"
        if value of attribute "AXMain" of targetWindow is not true then error "Dedicated window is not main"
        set targetArea to scroll area 1 of splitter group 1 of targetWindow
        set targetText to text area 1 of targetArea
        {action}
    end tell
end tell
"""
    try:
        return subprocess.run(
            ["osascript", "-"], input=source, text=True, capture_output=True,
            check=True, timeout=10,
        ).stdout.rstrip("\n")
    except subprocess.CalledProcessError as error:
        raise RuntimeError(f"scoped Terminal GUI action failed: {error.stderr.strip()}") from error


def selection_range(text, selected):
    offset = text.find(selected)
    if offset < 0:
        raise RuntimeError("known restored text is absent from the native text area")
    return len(text[:offset].encode("utf-16-le")) // 2, len(selected.encode("utf-16-le")) // 2


def copy_check(window, tty, target):
    expected = COPY_TARGETS[target]
    text = gui(window, tty, "return value of targetText")
    offset, length = selection_range(text, expected)
    helper = Path(__file__).parent / "artifacts" / "select-range"
    name = script(window, tty, f"name of window id {window}")
    selected_result = subprocess.run(
        [str(helper), str(offset), str(length), name, expected],
        text=True, capture_output=True, timeout=10,
    )
    if selected_result.returncode:
        raise RuntimeError(f"native range selection failed: {selected_result.stderr.strip()}")
    subprocess.run(
        ["pbcopy"], input="CLIMON2-52-CLIPBOARD-PENDING", text=True, check=True, timeout=5,
    )
    gui(window, tty, 'keystroke "c" using command down')
    copied = subprocess.run(
        ["pbpaste"], text=True, capture_output=True, check=True, timeout=5,
    ).stdout
    if copied != expected:
        raise RuntimeError("clipboard did not contain exactly the synthetic selected text; contents not recorded")
    return {"target": target, "native_selection": True, "clipboard_exact_match": True}


def input_check(window, tty):
    directory = Path(__file__).parent
    report = directory / "artifacts" / "native-interactive-input.json"
    if report.exists():
        raise RuntimeError("refusing to overwrite an existing native input report")
    busy = script(window, tty, "busy of testTab")
    if busy != "false":
        raise RuntimeError("dedicated tab is already running a program")
    accepted_before = script(window, tty, "history of testTab").count("INPUT-52-ACCEPTED")
    command = shlex.join([
        sys.executable, str(directory / "interactive_input.py"),
        "--permission-confirmed", "--expected-tty", tty, "--report-file", str(report),
    ])
    source = f"""
tell application "Terminal"
    set testTab to tab 1 of window id {window}
    if tty of testTab is not "{tty}" then error "Test tab identity changed"
    do script {quote(command)} in testTab
end tell
"""
    subprocess.run(
        ["osascript", "-"], input=source, text=True, capture_output=True,
        check=True, timeout=10,
    )
    deadline = time.monotonic() + 10
    while "INPUT-52-READY" not in script(window, tty, "history of testTab"):
        if time.monotonic() >= deadline:
            raise RuntimeError("interactive fixture did not reach its ready checkpoint")
        time.sleep(0.1)
    position = float(gui(
        window, tty,
        'set value of scroll bar 1 of targetArea to 0.0\nreturn value of scroll bar 1 of targetArea',
    ))
    if position >= 1:
        raise RuntimeError("native viewport did not scroll away from the bottom")
    gui(window, tty, 'keystroke "INPUT-52-SYNTHETIC"\nkey code 36')
    deadline = time.monotonic() + 10
    while (
        not report.exists()
        or script(window, tty, "history of testTab").count("INPUT-52-ACCEPTED") <= accepted_before
    ):
        if time.monotonic() >= deadline:
            raise RuntimeError("native GUI input was not received by the interactive fixture")
        time.sleep(0.1)
    received = json.loads(report.read_text())
    if received.get("received") != "INPUT-52-SYNTHETIC" or not received.get("input_matched"):
        raise RuntimeError("native input report does not match the synthetic fixture")
    return {
        "native_scrollbar_before_input": position,
        "input_received_while_scrolled": True,
        "canonical_mode": received["canonical_mode"],
        "echo_enabled": received["echo_enabled"],
        "boundary": "ordinary fixture input after synthetic restoration; no daemon/client integration",
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--permission-confirmed", action="store_true", required=True)
    parser.add_argument("--clipboard-permission-confirmed", action="store_true")
    parser.add_argument("--window-id", type=int, required=True)
    parser.add_argument("--tty", required=True)
    parser.add_argument("--check", choices=[*COPY_TARGETS, "input"], required=True)
    args = parser.parse_args()
    suffix = args.tty.removeprefix("/dev/ttys")
    if args.window_id <= 0 or not args.tty.startswith("/dev/ttys") or len(suffix) != 3 or not suffix.isascii() or not suffix.isdecimal():
        parser.error("require a dedicated numeric window ID and /dev/ttysNNN")
    if args.check == "input":
        result = input_check(args.window_id, args.tty)
    else:
        if not args.clipboard_permission_confirmed:
            parser.error("copy check requires explicit clipboard overwrite/read permission")
        result = copy_check(args.window_id, args.tty, args.check)
    print(json.dumps(result, ensure_ascii=True, indent=2))


if __name__ == "__main__":
    main()
