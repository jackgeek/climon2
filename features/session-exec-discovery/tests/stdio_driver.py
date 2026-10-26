"""Redirect selected handles while retaining the harness-owned terminal."""
import contextlib
import json
from pathlib import Path
import subprocess
import sys

request = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
with contextlib.ExitStack() as stack:
    streams = {}
    for name in ("stdin", "stdout", "stderr"):
        if name in request:
            streams[name] = subprocess.PIPE if request[name] == "PIPE" else stack.enter_context(open(request[name], "rb" if name == "stdin" else "wb"))
    child = subprocess.Popen(request["argv"], **streams)
    if streams.get("stdin") == subprocess.PIPE:
        child.stdin.close()
    if "pid_file" in request:
        Path(request["pid_file"]).write_text(str(child.pid))
    try:
        code = child.wait(timeout=120)
    finally:
        if child.poll() is None:
            child.kill()
            child.wait(timeout=15)
sys.exit(code if code >= 0 else 128 - code)
