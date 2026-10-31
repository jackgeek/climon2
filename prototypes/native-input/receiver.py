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


def publish(path, value):
    temporary = path.with_suffix(".tmp")
    with temporary.open("x") as output:
        json.dump(value, output, indent=2)
        output.write("\n")
    temporary.rename(path)


def receive(fd, expected):
    received = bytearray()
    deadline = time.monotonic() + 8
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


def run(expected_tty, directory):
    fd = sys.stdin.fileno()
    if not os.isatty(fd) or os.ttyname(fd) != expected_tty:
        raise RuntimeError("receiver is not on its expected test TTY")
    if (directory / "result.json").exists() or any(directory.glob("ready-*.json")):
        raise RuntimeError("receiver output paths must be unused")
    saved = termios.tcgetattr(fd)
    report = {"status": "FAIL", "stages": [], "tty": expected_tty}
    try:
        tty.setraw(fd)
        for index, (name, modes, key_code, expected) in enumerate(STAGES):
            sys.stdout.buffer.write(modes)
            sys.stdout.buffer.flush()
            publish(directory / f"ready-{index}.json", {
                "stage": name, "key_code": key_code, "tty": expected_tty,
            })
            received = receive(fd, expected)
            matched = received == expected
            report["stages"].append({
                "stage": name, "expected_hex": expected.hex(),
                "matched": matched, "received_length": len(received),
            })
            if not matched:
                raise RuntimeError(f"{name}: native key did not match the expected bytes; unexpected contents not recorded")
        report["status"] = "PASS"
    finally:
        sys.stdout.buffer.write(RESET)
        sys.stdout.buffer.flush()
        termios.tcsetattr(fd, termios.TCSANOW, saved)
        publish(directory / "result.json", report)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--expected-tty", required=True)
    parser.add_argument("--report-dir", type=Path, required=True)
    args = parser.parse_args()
    run(args.expected_tty, args.report_dir)


if __name__ == "__main__":
    main()
