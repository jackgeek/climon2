"""Throwaway #52 bounded paste receiver, not product integration."""

import argparse
import json
import os
from pathlib import Path
import re
import select
import signal
import sys
import termios
import time
import tty


PAYLOAD = b"PASTE-52-SYNTHETIC"


class ModeUnavailable(RuntimeError):
    pass


def reported_mode(reply):
    match = re.fullmatch(rb"\x1b\[\?2004;([12])\$y", reply)
    if match is None:
        raise ModeUnavailable("terminal did not report a known mutable mode 2004")
    return match[1] == b"1"


def emit(data):
    sys.stdout.buffer.write(data)
    sys.stdout.buffer.flush()


def set_mode(enabled):
    emit(b"\x1b[?2004h" if enabled else b"\x1b[?2004l")


def read_bytes(fd, length, timeout, expected=None):
    data = bytearray()
    deadline = time.monotonic() + timeout
    while len(data) < length:
        remaining = deadline - time.monotonic()
        if remaining <= 0 or not select.select([fd], [], [], remaining)[0]:
            raise TimeoutError("bounded synthetic receiver timed out")
        chunk = os.read(fd, length - len(data))
        if not chunk:
            raise EOFError("synthetic receiver reached EOF")
        data.extend(chunk)
        if expected is not None and not expected.startswith(data):
            raise RuntimeError("paste bytes do not match the synthetic stage; contents not recorded")
    return bytes(data)


def query_mode(fd):
    emit(b"\x1b[?2004$p")
    try:
        reply = read_bytes(fd, len(b"\x1b[?2004;2$y"), 3)
    except TimeoutError as error:
        raise ModeUnavailable("no bounded mode 2004 reply; stopped before changing it") from error
    return reported_mode(reply)


def receive_pastes(fd, attempt):
    for index in range(32):
        emit(f"PASTE-52-ROW-{index:03}\r\n".encode())
    set_mode(True)
    emit(f"ON-52-{attempt}\r\n".encode())
    bracketed = b"\x1b[200~" + PAYLOAD + b"\x1b[201~"
    read_bytes(fd, len(bracketed), 10, bracketed)
    set_mode(False)
    emit(f"OFF-52-{attempt}\r\n".encode())
    read_bytes(fd, len(PAYLOAD), 10, PAYLOAD)
    return {"status": "passed", "bracketed_on_exact": True, "bracketed_off_exact": True}


def interrupted(_signal, _frame):
    raise InterruptedError("bounded paste fixture interrupted")


def configured_attributes(fd):
    attributes = termios.tcgetattr(fd)
    # macOS sets PENDIN while returning to canonical input; it is transient state.
    attributes[3] &= ~termios.PENDIN
    return attributes


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--permission-confirmed", action="store_true", required=True)
    parser.add_argument("--expected-tty", required=True)
    parser.add_argument("--attempt", required=True)
    parser.add_argument("--report-file", type=Path, required=True)
    args = parser.parse_args()
    if re.fullmatch(r"[0-9a-f]{16}", args.attempt) is None:
        parser.error("attempt must contain exactly 16 lowercase hexadecimal characters")
    for stream in [sys.stdin, sys.stdout]:
        if not stream.isatty() or os.ttyname(stream.fileno()) != args.expected_tty:
            raise RuntimeError("paste fixture requires the authorized input/output TTY")
    if args.report_file.exists():
        raise RuntimeError("refusing to overwrite an existing paste report")
    fd = sys.stdin.fileno()
    attributes = termios.tcgetattr(fd)
    configured = configured_attributes(fd)
    original_mode = None
    signal.signal(signal.SIGALRM, interrupted)
    signal.signal(signal.SIGTERM, interrupted)
    signal.alarm(60)
    try:
        tty.setraw(fd, termios.TCSANOW)
        original_mode = query_mode(fd)
        result = receive_pastes(fd, args.attempt)
    except ModeUnavailable as error:
        result = {"status": "blocked", "reason": str(error)}
    finally:
        signal.alarm(0)
        try:
            if original_mode is not None:
                set_mode(original_mode)
        finally:
            termios.tcflush(fd, termios.TCIFLUSH)
            termios.tcsetattr(fd, termios.TCSANOW, attributes)
    if configured_attributes(fd) != configured:
        raise RuntimeError("original terminal input attributes were not restored")
    result.update({
        "attempt": args.attempt, "initial_bracketed_mode": original_mode,
        "input_modes_restored": True,
        "host_mode_restore_emitted": original_mode is not None,
        "boundary": "exact synthetic paste bytes; no client/daemon or mode capture implementation",
    })
    with args.report_file.open("x") as output:
        json.dump(result, output, indent=2)
        output.write("\n")
    marker = "OK" if result["status"] == "passed" else "NO"
    emit(f"{marker}-52-{args.attempt}\r\n".encode())
    return 0 if result["status"] == "passed" else 2


if __name__ == "__main__":
    sys.exit(main())
