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
SAMPLES = {
    "ascii": PAYLOAD,
    "multiline-unicode": "# PASTE-52-\u754ce\u0301\n# SECOND-52".encode("utf-8"),
}


def payload_for(sample):
    return SAMPLES[sample]


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


def read_bytes(fd, length, timeout):
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
    return bytes(data)


def query_mode(fd):
    emit(b"\x1b[?2004$p")
    try:
        reply = read_bytes(fd, len(b"\x1b[?2004;2$y"), 3)
    except TimeoutError as error:
        raise ModeUnavailable("no bounded mode 2004 reply; stopped before changing it") from error
    return reported_mode(reply)


def paste_frames(payload, bracketed):
    variants = {"none": payload}
    if b"\n" in payload:
        variants = {
            "LF": payload, "CR": payload.replace(b"\n", b"\r"),
            "CRLF": payload.replace(b"\n", b"\r\n"),
        }
    prefix, suffix = (b"\x1b[200~", b"\x1b[201~") if bracketed else (b"", b"")
    return {prefix + data + suffix: newline for newline, data in variants.items()}


def observe_paste(fd, payload, bracketed):
    frames = paste_frames(payload, bracketed)
    data = b""
    deadline = time.monotonic() + 10
    while True:
        data += read_bytes(fd, 1, deadline - time.monotonic())
        if data in frames:
            return frames[data]
        if not any(frame.startswith(data) for frame in frames):
            raise RuntimeError("paste bytes are not a known synthetic variant; contents not recorded")


def receive_pastes(fd, attempt, sample="ascii"):
    payload = payload_for(sample)
    for index in range(32):
        emit(f"PASTE-52-ROW-{index:03}\r\n".encode())
    set_mode(True)
    emit(f"ON-52-{attempt}\r\n".encode())
    on_newline = observe_paste(fd, payload, True)
    set_mode(False)
    emit(f"OFF-52-{attempt}\r\n".encode())
    off_newline = observe_paste(fd, payload, False)
    return {
        "status": "passed", "sample": sample, "payload_utf8_preserved": True,
        "bracketed_on_exact": True, "bracketed_on_newline": on_newline,
        "bracketed_off_exact": True, "bracketed_off_newline": off_newline,
    }


def interrupted(_signal, _frame):
    raise InterruptedError("bounded paste fixture interrupted")


def configured_attributes(fd):
    attributes = termios.tcgetattr(fd)
    # macOS sets PENDIN while returning to canonical input; it is transient state.
    attributes[3] &= ~termios.PENDIN
    return attributes


def run_session(fd, attempt, off_baseline, sample="ascii"):
    attributes = termios.tcgetattr(fd)
    configured = configured_attributes(fd)
    restoration_target = None
    signal.signal(signal.SIGALRM, interrupted)
    signal.signal(signal.SIGTERM, interrupted)
    signal.alarm(60)
    try:
        tty.setraw(fd, termios.TCSANOW)
        if off_baseline:
            restoration_target = False
            set_mode(False)
        else:
            restoration_target = query_mode(fd)
        result = receive_pastes(fd, attempt, sample)
    except ModeUnavailable as error:
        result = {"status": "blocked", "reason": str(error)}
    finally:
        signal.alarm(0)
        try:
            if restoration_target is not None:
                set_mode(restoration_target)
        finally:
            try:
                termios.tcflush(fd, termios.TCIFLUSH)
            finally:
                termios.tcsetattr(fd, termios.TCSANOW, attributes)
    if configured_attributes(fd) != configured:
        raise RuntimeError("original terminal input attributes were not restored")
    result.update({
        "attempt": attempt, "restoration_target_bracketed_mode": restoration_target,
        "state_source": "permissioned-off-baseline" if off_baseline else "query",
        "input_modes_restored": True,
        "host_mode_restore_emitted": restoration_target is not None,
        "boundary": "exact synthetic paste bytes; no client/daemon or mode capture implementation",
    })
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--permission-confirmed", action="store_true", required=True)
    modes = parser.add_mutually_exclusive_group(required=True)
    modes.add_argument("--mode-query-permission-confirmed", action="store_true")
    modes.add_argument("--off-baseline-permission-confirmed", action="store_true")
    parser.add_argument("--expected-tty", required=True)
    parser.add_argument("--attempt", required=True)
    parser.add_argument("--report-file", type=Path, required=True)
    parser.add_argument("--sample", choices=SAMPLES, default="ascii")
    args = parser.parse_args()
    if re.fullmatch(r"[0-9a-f]{16}", args.attempt) is None:
        parser.error("attempt must contain exactly 16 lowercase hexadecimal characters")
    for stream in [sys.stdin, sys.stdout]:
        if not stream.isatty() or os.ttyname(stream.fileno()) != args.expected_tty:
            raise RuntimeError("paste fixture requires the authorized input/output TTY")
    if args.report_file.exists():
        raise RuntimeError("refusing to overwrite an existing paste report")
    result = run_session(
        sys.stdin.fileno(), args.attempt, args.off_baseline_permission_confirmed, args.sample,
    )
    with args.report_file.open("x") as output:
        json.dump(result, output, indent=2)
        output.write("\n")
    marker = "OK" if result["status"] == "passed" else "NO"
    emit(f"{marker}-52-{args.attempt}\r\n".encode())
    return 0 if result["status"] == "passed" else 2


if __name__ == "__main__":
    sys.exit(main())
