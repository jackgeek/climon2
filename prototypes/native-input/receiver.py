"""Throwaway #56 raw-PTY native-key receiver. No host queries or clipboard."""

import argparse
import json
import os
from pathlib import Path
import select
import sys
import termios
import time
import tty


STAGES = [
    ("cursor-normal", b"\x1b[?1l\x1b>", 126, b"\x1b[A"),
    ("cursor-application", b"\x1b[?1h", 126, b"\x1bOA"),
    ("cursor-handoff-normal", b"\x1b[?1l", 126, b"\x1b[A"),
    ("keypad-normal", b"\x1b>", 83, b"1"),
    ("keypad-application", b"\x1b=", 83, b"\x1bOq"),
    ("keypad-handoff-normal", b"\x1b>", 83, b"1"),
]
RESET = b"\x1b[?1l\x1b>"
ASCII_PASTE = "# PASTE-56-ASCII"
UNICODE_PASTE = "# PASTE-56-\u754ce\u0301\n# SECOND-56"
UNICODE_VARIANTS = tuple(
    UNICODE_PASTE.replace("\n", newline).encode("utf-8")
    for newline in ("\n", "\r", "\r\n")
)
PASTE_STAGES = [
    ("paste-ascii-on", b"\x1b[?2004l\x1b[?2004h", ASCII_PASTE,
     b"\x1b[200~# PASTE-56-ASCII\x1b[201~"),
    ("paste-ascii-off", b"\x1b[?2004l", ASCII_PASTE, b"# PASTE-56-ASCII"),
    ("paste-unicode-on", b"\x1b[?2004h", UNICODE_PASTE,
     tuple(b"\x1b[200~" + value + b"\x1b[201~" for value in UNICODE_VARIANTS)),
    ("paste-unicode-off", b"\x1b[?2004l", UNICODE_PASTE, UNICODE_VARIANTS),
]


def publish(path, value):
    temporary = path.with_suffix(".tmp")
    with temporary.open("x") as output:
        json.dump(value, output, indent=2)
        output.write("\n")
    temporary.rename(path)


def receive(fd, expected, timeout_seconds):
    received = bytearray()
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        wait = deadline - time.monotonic()
        if len(received) >= len(expected):
            wait = min(wait, 0.1)
        if not select.select([fd], [], [], max(0, wait))[0]:
            break
        chunk = os.read(fd, 64)
        if not chunk:
            break
        received.extend(chunk)
        if len(received) > 64:
            break
    return bytes(received)


def run(expected_tty, directory, stages=STAGES, input_timeout_seconds=8, start_file=None,
        collect_all_stages=False):
    fd = sys.stdin.fileno()
    if not os.isatty(fd) or os.ttyname(fd) != expected_tty:
        raise RuntimeError("receiver is not on its expected test TTY")
    if (directory / "result.json").exists() or any(directory.glob("ready-*.json")):
        raise RuntimeError("receiver output paths must be unused")
    saved = termios.tcgetattr(fd)
    report = {"status": "FAIL", "stages": [], "tty": expected_tty}
    try:
        tty.setraw(fd)
        for index, (name, modes, action, expected) in enumerate(stages):
            variants = (expected,) if isinstance(expected, bytes) else expected
            sys.stdout.buffer.write(modes)
            sys.stdout.buffer.flush()
            publish(directory / f"ready-{index}.json", {
                "stage": name, "tty": expected_tty,
                **({"key_code": action} if isinstance(action, int) else {"paste_text": action}),
            })
            if index == 0 and start_file is not None:
                deadline = time.monotonic() + 600
                while not start_file.exists():
                    if time.monotonic() >= deadline:
                        raise TimeoutError("Receiver start confirmation expired")
                    time.sleep(0.02)
            received = receive(fd, variants[0], input_timeout_seconds)
            matched = received in variants
            report["stages"].append({
                "stage": name, "expected_hex": variants[0].hex(),
                "matched": matched, "received_length": len(received),
                "matched_expected_hex": received.hex() if matched else None,
            })
            if not matched and not collect_all_stages:
                raise RuntimeError(f"{name}: native input did not match the expected bytes; unexpected contents not recorded")
        if any(not stage["matched"] for stage in report["stages"]):
            raise RuntimeError("Native input did not match in one or more stages; unexpected contents not recorded")
        report["status"] = "PASS"
    finally:
        sys.stdout.buffer.write(RESET)
        if any(isinstance(stage[2], str) for stage in stages):
            sys.stdout.buffer.write(b"\x1b[?2004l")
        sys.stdout.buffer.flush()
        termios.tcsetattr(fd, termios.TCSANOW, saved)
        publish(directory / "result.json", report)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--expected-tty", required=True)
    parser.add_argument("--report-dir", type=Path, required=True)
    parser.add_argument("--suite", choices=["keys", "paste", "vscode", "cursor-normal"], default="keys")
    parser.add_argument("--input-timeout-seconds", type=int, default=8)
    parser.add_argument("--start-file", type=Path)
    parser.add_argument("--collect-all-stages", action="store_true")
    args = parser.parse_args()
    if not 1 <= args.input_timeout_seconds <= 600:
        parser.error("--input-timeout-seconds must be between 1 and 600")
    if args.start_file is not None and args.start_file.exists():
        parser.error("--start-file must be unused")
    expected_tty = os.ttyname(sys.stdin.fileno()) if args.expected_tty == "self" else args.expected_tty
    stages = {"keys": STAGES, "paste": PASTE_STAGES, "vscode": STAGES + PASTE_STAGES,
              "cursor-normal": STAGES[:1]}
    run(expected_tty, args.report_dir, stages[args.suite], args.input_timeout_seconds, args.start_file,
        args.collect_all_stages)


if __name__ == "__main__":
    main()
