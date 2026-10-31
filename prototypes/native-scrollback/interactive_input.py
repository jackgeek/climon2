"""Throwaway #52 ordinary-input fixture; synthetic data only."""

import argparse
import json
import os
from pathlib import Path
import signal
import sys
import termios


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--permission-confirmed", action="store_true", required=True)
    parser.add_argument("--expected-tty", required=True)
    parser.add_argument("--report-file", type=Path, required=True)
    args = parser.parse_args()
    if not sys.stdin.isatty() or os.ttyname(sys.stdin.fileno()) != args.expected_tty:
        raise RuntimeError("ordinary-input fixture must run on the authorized TTY")
    if args.report_file.exists():
        raise RuntimeError("refusing to overwrite an existing input report")
    flags = termios.tcgetattr(sys.stdin.fileno())[3]
    canonical = bool(flags & termios.ICANON)
    echo = bool(flags & termios.ECHO)
    if not canonical or not echo:
        raise RuntimeError("fixture requires ordinary canonical, echoed terminal input")
    for index in range(32):
        print(f"SCROLL-52-{index:03}")
    print("INPUT-52-READY", flush=True)
    signal.signal(signal.SIGALRM, input_timeout)
    signal.alarm(60)
    try:
        line = sys.stdin.readline(64)
    finally:
        signal.alarm(0)
    if line != "INPUT-52-SYNTHETIC\n":
        raise RuntimeError("received input does not match the synthetic fixture line")
    report = {
        "input_matched": True,
        "canonical_mode": canonical,
        "echo_enabled": echo,
        "received": "INPUT-52-SYNTHETIC",
        "boundary": "ordinary input; native scrolling must be established by the host observer",
    }
    with args.report_file.open("x") as output:
        json.dump(report, output, indent=2)
        output.write("\n")
    print("INPUT-52-ACCEPTED", flush=True)


def input_timeout(_signal, _frame):
    raise TimeoutError("no synthetic input received within 60 seconds")


if __name__ == "__main__":
    main()
