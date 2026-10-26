import contextlib
import datetime as dt
import json
import os
from pathlib import Path
import re
import struct
import subprocess
import threading
import time
import uuid

from harness import DEFAULT_TIMEOUT, pid_alive, wait_until
from afunix import LocalSocket, frame, receive_frame
from native import WINDOWS, TRACKED_PIDS, assert_detached, assert_private, descendants, process_start, processes

GUID = re.compile(r"[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}\Z")
STAMP = re.compile(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z\Z")
HELLO = b'{"type":"hello","protocol":1}'
# Client-side observations include accept scheduling and remote-close delivery.
OBSERVATION_GRACE = 0.75 if WINDOWS else 0.25
FIELDS = {"format_version", "session_id", "socket_path", "daemon_pid", "daemon_start_time", "command", "args", "cwd", "depth", "created_at", "climon2_version"}


def run(install, args, env, cwd=None, timeout=DEFAULT_TIMEOUT):
    return subprocess.run([str(install.launcher), *args], input=b"", capture_output=True, env=env, cwd=cwd, timeout=timeout)


def error(result, fragment, path=None):
    assert result.returncode == 125, f"climon2-owned failure must return 125, got {result.returncode}: {result.stderr!r}"
    assert result.stdout == b"", "climon2-owned failure must not write stdout"
    stderr = result.stderr.decode("utf-8")
    assert stderr.startswith("climon2: "), f"missing climon2 diagnostic prefix: {stderr!r}"
    assert fragment in stderr, f"missing diagnostic {fragment!r}: {stderr!r}"
    if path is not None:
        expected_path = str(path).casefold() if WINDOWS else str(path)
        diagnostic = stderr.casefold() if WINDOWS else stderr
        assert expected_path in diagnostic, f"diagnostic must name affected path {path}: {stderr!r}"


def listed(install, env, cwd=None):
    result = run(install, ["list", "--json"], env, cwd)
    assert result.returncode == 0, f"list --json must succeed: {result.stderr!r}"
    assert result.stdout.endswith(b"\n"), "JSON listing must end with a newline"
    value = json.loads(result.stdout)
    assert isinstance(value, list), "JSON listing must be an array"
    return value


def ready(term):
    report = json.loads(term.expect(r"FIXTURE READY \S+ (\{[^\n]*\})\n").group(1))
    TRACKED_PIDS.add(report["pid"])
    TRACKED_PIDS.update(descendants(report["pid"]))
    return report


def answer(term, command, prefix):
    term.send_line(command)
    return json.loads(term.expect(r"(?:^|\n)" + prefix + r" ([^\n]*)\n").group(1))


def record_for(env, session_id=None, cwd=None):
    home = Path(env["CLIMON2_HOME"])
    if not home.is_absolute():
        home = Path(cwd or os.getcwd()) / home
    directory = home / "sessions"
    def find():
        files = list(directory.glob(f"{session_id or '*'}.pty"))
        return files[0] if len(files) == 1 else None
    path = wait_value(find, "session metadata was not published")
    record = json.loads(path.read_text(encoding="utf-8"))
    TRACKED_PIDS.add(record["daemon_pid"])
    return path, record


def wait_value(fn, message, timeout=DEFAULT_TIMEOUT):
    value = []
    def check():
        candidate = fn()
        if candidate:
            value.append(candidate)
            return True
        return False
    wait_until(check, message, timeout=timeout)
    return value[-1]


def validate_record(path, record, install, command, args, cwd, depth=1):
    assert FIELDS <= record.keys(), "metadata must contain every B11 field"
    sid = record["session_id"]
    assert GUID.fullmatch(sid), "session GUID must be lowercase canonical hexadecimal"
    assert path.name == sid + ".pty", "metadata filename must match session GUID"
    assert type(record["format_version"]) is int and record["format_version"] == 1
    assert type(record["daemon_pid"]) is int and record["daemon_pid"] > 0
    assert type(record["depth"]) is int and record["depth"] == depth
    assert record["command"] == command
    assert record["args"] == args
    assert Path(record["cwd"]).is_absolute() and Path(record["cwd"]).resolve() == Path(cwd).resolve()
    assert record["climon2_version"] == install.version
    assert Path(record["socket_path"]).is_absolute()
    for field in ("created_at", "daemon_start_time"):
        assert STAMP.fullmatch(record[field]), f"{field} must be UTC RFC3339 with Z"
        dt.datetime.fromisoformat(record[field].replace("Z", "+00:00"))
    started = dt.datetime.fromisoformat(record["daemon_start_time"].replace("Z", "+00:00")).timestamp()
    assert abs(started - process_start(record["daemon_pid"])) < 1.1, "daemon_start_time must describe the actual process start"
    created = dt.datetime.fromisoformat(record["created_at"].replace("Z", "+00:00")).timestamp()
    assert started - DEFAULT_TIMEOUT <= created <= time.time() + 1, "created_at must describe this session's creation, not an unrelated timestamp"
    assert_detached(record["daemon_pid"])
    assert_private(path, 0o600)


def cleaned(path, record, command_pid=None, remaining=()):
    pids = [record["daemon_pid"], *remaining]
    if command_pid is not None:
        pids.append(command_pid)
    TRACKED_PIDS.update(pids)
    wait_until(lambda: not path.exists() and not Path(record["socket_path"]).exists() and all(not pid_alive(pid) for pid in pids),
               f"session cleanup must remove metadata/socket and terminate processes {pids}")


def no_state(env, install):
    home = Path(env["CLIMON2_HOME"])
    wait_until(lambda: not list((home / "sessions").glob("*.pty")), "failed startup left published metadata")
    runtime = Path(env["LOCALAPPDATA"]) / "climon2" / "run" if WINDOWS else Path(env["XDG_RUNTIME_DIR"])
    assert not runtime.exists() or not any(not item.is_dir() for item in runtime.rglob("*")), "failed startup left a socket or temporary file"
    def no_companions():
        return not any(str(install.root).casefold() in info["exe"].casefold() and "climon2" in Path(info["exe"]).name for info in processes().values())
    wait_until(no_companions, "failed startup leaked CLI, client or session daemon")


def hello(path, payload=HELLO, fragmented=False):
    with LocalSocket() as sock:
        start = time.monotonic()
        sock.connect(path)
        data = frame(payload)
        if fragmented:
            timer = threading.Event()
            for index, part in enumerate((data[:1], data[1:3], data[3:8], data[8:])):
                if index:
                    timer.wait(0.15)
                sock.sendall(part)
        else:
            sock.sendall(data)
        reply = json.loads(receive_frame(sock, start + 2))
        return reply


def assert_hello(record, **kwargs):
    assert hello(record["socket_path"], **kwargs) == {"type": "hello", "protocol": 1, "session_id": record["session_id"]}, "authorized protocol-1 hello must identify the live session"


class Endpoint:
    def __init__(self, path, session_id, mode="live"):
        self.path, self.session_id, self.mode = Path(path), session_id, mode
        self.stop = threading.Event()
        self.accepted = threading.Event()
        self.connections = []
        self.workers = []
        self.errors = []
        self.requests = []
        self.socket = LocalSocket()
        self.socket.bind(path)
        self.socket.listen()
        self.socket.settimeout(0.1)
        self.thread = threading.Thread(target=self._accept, daemon=True)
        self.thread.start()

    def _accept(self):
        while not self.stop.is_set():
            try:
                conn = self.socket.accept()
            except TimeoutError:
                continue
            except OSError as exc:
                if not self.stop.is_set():
                    self.errors.append(exc)
                return
            self.connections.append(conn)
            self.accepted.set()
            worker = threading.Thread(target=self._serve, args=(conn,), daemon=True)
            self.workers.append(worker)
            worker.start()

    def _serve(self, conn):
        try:
            request = json.loads(receive_frame(conn, time.monotonic() + 2))
            self.requests.append(request)
            if request != {"type": "hello", "protocol": 1}:
                self.errors.append(AssertionError(f"discovery must send a protocol-1 hello, got {request!r}"))
                return
            if self.mode == "silent":
                self.stop.wait(DEFAULT_TIMEOUT)
                return
            reply = {"type": "hello", "protocol": 2 if self.mode == "protocol" else 1,
                     "session_id": str(uuid.uuid4()) if self.mode == "mismatch" else self.session_id}
            payload = b"not JSON" if self.mode == "malformed" else json.dumps(reply).encode()
            wire = frame(payload)
            if self.mode == "partial":
                conn.sendall(wire[:6])
                self.stop.wait(DEFAULT_TIMEOUT)
            elif self.mode == "drip":
                for byte in wire:
                    conn.sendall(bytes([byte]))
                    if self.stop.wait(0.35):
                        return
            else:
                if self.mode == "delayed" and self.stop.wait(1.2):
                    return
                conn.sendall(wire)
        except (OSError, EOFError, TimeoutError) as exc:
            if self.mode not in ("drip", "partial", "silent") and not self.stop.is_set():
                self.errors.append(exc)
        finally:
            conn.close()

    def close(self):
        self.stop.set()
        self.thread.join(1)
        self.socket.close()
        for conn in self.connections:
            conn.close()
        for worker in self.workers:
            worker.join(1)
        self.path.unlink(missing_ok=True)
        assert not self.thread.is_alive() and all(not worker.is_alive() for worker in self.workers), "fake endpoint thread did not stop"
        assert not self.errors, f"fake endpoint failed: {self.errors}"


@contextlib.contextmanager
def endpoint(directory, session_id=None, mode="live"):
    ep = Endpoint(directory / (uuid.uuid4().hex[:12] + ".sock"), session_id or str(uuid.uuid4()), mode)
    try:
        yield ep
    finally:
        ep.close()


def synthetic(home, ep, **changes):
    record = {"format_version": 1, "session_id": ep.session_id, "socket_path": str(ep.path),
              "daemon_pid": os.getpid(), "daemon_start_time": "2024-01-01T00:00:00Z",
              "command": "synthetic-command", "args": ["a b"], "cwd": str(home.resolve()),
              "depth": 3, "created_at": "2024-01-01T00:00:00Z", "climon2_version": "0.1.0"}
    record.update(changes)
    directory = home / "sessions"
    directory.mkdir(parents=True, exist_ok=True)
    path = directory / (record["session_id"] + ".pty")
    path.write_text(json.dumps(record, ensure_ascii=True), encoding="utf-8")
    return path, record
