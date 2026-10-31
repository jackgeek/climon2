"""Permissioned #52 native paste oracle; one dedicated Terminal.app tab."""

import argparse
import json
from pathlib import Path
import shlex
import subprocess
import sys
import time
import uuid

from paste_input import PAYLOAD
from terminal_app import script, validate_target
from terminal_gui import gui, quote


def wait_stage(window, tty, attempt, stage, report):
    deadline = time.monotonic() + 10
    while True:
        history = script(window, tty, "history of testTab")
        if f"NO-52-{attempt}" in history:
            return json.loads(report.read_text())
        if f"{stage}-52-{attempt}" in history:
            return None
        if time.monotonic() >= deadline:
            raise TimeoutError("fresh native paste checkpoint was not observed")
        time.sleep(0.1)


def wait_idle(window, tty):
    deadline = time.monotonic() + 65
    while script(window, tty, "busy of testTab") != "false":
        if time.monotonic() >= deadline:
            raise TimeoutError("paste receiver did not exit; native cleanup remains unverified")
        time.sleep(0.1)


def paste_stages(window, tty, attempt, report):
    for stage in ["ON", "OFF"]:
        blocked = wait_stage(window, tty, attempt, stage, report)
        if blocked is not None:
            return blocked
        subprocess.run(["pbcopy"], input=PAYLOAD, check=True, timeout=5)
        gui(window, tty, f"""
tell application "Terminal"
    if busy of tab 1 of window id {window} is not true then error "Paste receiver exited before input"
end tell
keystroke "v" using command down
""")
    blocked = wait_stage(window, tty, attempt, "OK", report)
    return blocked if blocked is not None else json.loads(report.read_text())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--permission-confirmed", action="store_true", required=True)
    parser.add_argument("--clipboard-permission-confirmed", action="store_true", required=True)
    parser.add_argument("--mode-query-permission-confirmed", action="store_true", required=True)
    parser.add_argument("--window-id", type=int, required=True)
    parser.add_argument("--tty", required=True)
    args = parser.parse_args()
    validate_target(args.window_id, args.tty)
    if script(args.window_id, args.tty, "busy of testTab") != "false":
        raise RuntimeError("dedicated tab is already running a program")
    geometry = script(
        args.window_id, args.tty,
        '(number of columns of testTab as text) & "," & (number of rows of testTab as text)',
    )
    if geometry != "24,6":
        raise RuntimeError("paste fixture requires the authorized 24x6 tab")
    if script(args.window_id, args.tty, "name of current settings of testTab") != "Basic":
        raise RuntimeError("dedicated tab no longer uses restored Basic")
    gui(args.window_id, args.tty, 'return "scoped"')
    attempt = uuid.uuid4().hex[:16]
    directory = Path(__file__).parent
    report = directory / "artifacts" / f"native-paste-{attempt}.json"
    if report.exists() or attempt in script(args.window_id, args.tty, "history of testTab"):
        raise RuntimeError("attempt already exists; refusing an ambiguous replay")
    command = shlex.join([
        sys.executable, str(directory / "paste_input.py"), "--permission-confirmed",
        "--expected-tty", args.tty, "--attempt", attempt, "--report-file", str(report),
    ])
    source = f"""
tell application "Terminal"
    set testTab to tab 1 of window id {args.window_id}
    if tty of testTab is not "{args.tty}" then error "Dedicated TTY changed"
    if busy of testTab is not false then error "Dedicated tab became busy"
    do script {quote(command)} in testTab
end tell
"""
    subprocess.run(
        ["osascript", "-"], input=source, text=True, capture_output=True,
        check=True, timeout=10,
    )
    try:
        result = paste_stages(args.window_id, args.tty, attempt, report)
    finally:
        wait_idle(args.window_id, args.tty)
    if result.get("attempt") != attempt or result.get("input_modes_restored") is not True:
        raise RuntimeError("native receiver report identity or cleanup does not match")
    result["native_cmd_v"] = result["status"] == "passed"
    result["clipboard_writes"] = 2 if result["status"] == "passed" else 0
    print(json.dumps(result, indent=2))
    return 0 if result["status"] == "passed" else 2


if __name__ == "__main__":
    sys.exit(main())
