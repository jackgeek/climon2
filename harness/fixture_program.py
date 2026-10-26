"""The generic terminal fixture: a small interactive program to run inside a session.

Usage: python fixture_program.py [--marker TEXT] [args…]

On start it prints `FIXTURE READY <marker> <json>` with its pid, whether stdin/stdout/stderr
are terminals, and the terminal size. Then it reads commands, one per line, and answers each
with one line. All values are JSON with ASCII escapes, so output is the same on every platform:

  size            SIZE <cols>x<rows>
  tty             TTY <json {"stdin": bool, "stdout": bool, "stderr": bool}>
  ctty            CTTY <json bool>: whether the controlling terminal (/dev/tty, CONOUT$) opens
  argv            ARGV <json list of the extra args>
  cwd             CWD <json path>
  env NAME        ENV <json {"NAME": value or null}>
  echo TEXT       ECHO <json TEXT>
  exit N          exits with code N
  anything else   UNKNOWN <json line>
"""

import json
import os
import shutil
import sys


def size() -> str:
    try:
        cols, rows = os.get_terminal_size(sys.stdout.fileno())
    except OSError:
        cols, rows = shutil.get_terminal_size((0, 0))
    return f"{cols}x{rows}"


def controlling_terminal() -> bool:
    try:
        with open("CONOUT$" if sys.platform == "win32" else "/dev/tty", "w"):
            return True
    except OSError:
        return False


def say(line: str) -> None:
    sys.stdout.write(line + "\n")
    sys.stdout.flush()


def ascii_json(value: object) -> str:
    return json.dumps(value, ensure_ascii=True)


def main(argv: list[str]) -> int:
    marker = "fixture"
    if argv[:1] == ["--marker"] and len(argv) >= 2:
        marker, argv = argv[1], argv[2:]
    ttys = {
        "stdin": sys.stdin.isatty(),
        "stdout": sys.stdout.isatty(),
        "stderr": sys.stderr.isatty(),
    }
    say(f"FIXTURE READY {marker} " + ascii_json({"pid": os.getpid(), "tty": ttys, "size": size()}))
    for raw in sys.stdin:
        line = raw.rstrip("\r\n")
        command, _, rest = line.partition(" ")
        if command == "size":
            say(f"SIZE {size()}")
        elif command == "tty":
            say(f"TTY {ascii_json(ttys)}")
        elif command == "ctty":
            say(f"CTTY {ascii_json(controlling_terminal())}")
        elif command == "argv":
            say(f"ARGV {ascii_json(argv)}")
        elif command == "cwd":
            say(f"CWD {ascii_json(os.getcwd())}")
        elif command == "env":
            say(f"ENV {ascii_json({rest: os.environ.get(rest)})}")
        elif command == "echo":
            say(f"ECHO {ascii_json(rest)}")
        elif command == "exit":
            return int(rest or 0)
        elif line:
            say(f"UNKNOWN {ascii_json(line)}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
