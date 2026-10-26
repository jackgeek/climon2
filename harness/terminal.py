"""A real terminal around a child process: a PTY on Unix, ConPTY on Windows."""

import codecs
import contextlib
import os
import re
import subprocess
import sys
import threading
import time
from collections.abc import Mapping, Sequence

DEFAULT_TIMEOUT = 15.0
"""Approved deadline (seconds) for any single readiness wait."""

_CURSOR_FORWARD = re.compile(r"\x1b\[(\d*)C")
_ESCAPES = re.compile(
    r"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)"  # OSC … BEL/ST
    r"|\x1b\[[0-?]*[ -/]*[@-~]"  # CSI
    r"|\x1b[PX^_][^\x1b]*\x1b\\"  # DCS/SOS/PM/APC … ST
    r"|\x1b[@-Z\\-_]"  # other two-byte escapes
    r"|\x1b[()][0-9A-Za-z]"  # charset selection
)


def clean(raw: str) -> str:
    """Terminal output as plain text: escape sequences and carriage returns removed, and
    cursor-forward moves (ConPTY's way of drawing spaces) turned back into spaces."""
    text = _CURSOR_FORWARD.sub(lambda m: " " * int(m.group(1) or 1), raw)
    return _ESCAPES.sub("", text).replace("\r", "")


class TerminalTimeout(AssertionError):
    pass


class Terminal:
    """Drive a program through a real terminal. Text sent is typed; output is accumulated."""

    def __init__(self, argv: Sequence[str], env: Mapping[str, str], cwd: str, cols: int, rows: int):
        self.argv = list(argv)
        self._lock = threading.Condition()
        self._raw = ""
        self._cursor = 0
        self._eof = False
        self._decoder = codecs.getincrementaldecoder("utf-8")(errors="replace")
        self._backend = _Windows(self) if sys.platform == "win32" else _Unix(self)
        self._backend.start(self.argv, dict(env), cwd, cols, rows)
        self._reader = threading.Thread(target=self._backend.read_loop, daemon=True)
        self._reader.start()

    @property
    def pid(self) -> int:
        return self._backend.pid

    @property
    def output(self) -> str:
        """Everything the program has written, raw (including escape sequences)."""
        with self._lock:
            return self._raw

    @property
    def text(self) -> str:
        """Everything the program has written, as plain text (see `clean`)."""
        return clean(self.output)

    def send(self, text: str) -> None:
        """Type `text` into the terminal. Use "\\r" for Enter."""
        self._backend.write(text)

    def send_line(self, text: str) -> None:
        self.send(text + "\r")

    def expect(
        self, pattern: "str | re.Pattern[str]", timeout: float = DEFAULT_TIMEOUT
    ) -> re.Match[str]:
        """Wait for `pattern` (a regex) in the plain-text output after the previous match.
        A string pattern is multiline: `^` and `$` match at line boundaries, so `^` also
        matches where the previous match ended on a newline."""
        regex = re.compile(pattern, re.MULTILINE) if isinstance(pattern, str) else pattern
        deadline = time.monotonic() + timeout
        with self._lock:
            while True:
                text = clean(self._raw)
                match = regex.search(text, self._cursor)
                if match:
                    self._cursor = match.end()
                    return match
                remaining = deadline - time.monotonic()
                if remaining <= 0 or (self._eof and not self.alive):
                    tail = text[self._cursor :][-2000:]
                    raise TerminalTimeout(
                        f"{regex.pattern!r} not seen within {timeout}s "
                        f"({'process ended' if not self.alive else 'still running'}); "
                        f"output since last match:\n{tail}"
                    )
                self._lock.wait(min(remaining, 0.1))

    def resize(self, cols: int, rows: int) -> None:
        self._backend.resize(cols, rows)

    @property
    def alive(self) -> bool:
        return self._backend.alive()

    def wait(self, timeout: float = DEFAULT_TIMEOUT) -> int:
        """Wait for the program to exit; return its exit code (negative signal number on Unix
        if it was killed by a signal). Output written before exit is collected first."""
        deadline = time.monotonic() + timeout
        while self.alive:
            if time.monotonic() > deadline:
                raise TerminalTimeout(
                    f"{self.argv[0]} still running after {timeout}s:\n{self.text[-2000:]}"
                )
            time.sleep(0.05)
        self._reader.join(timeout=1.0)
        return self._backend.exit_code()

    def close(self) -> None:
        """Kill the program if it is still running and release the terminal."""
        self._backend.close()
        self._reader.join(timeout=2.0)

    def _received(self, data: bytes | str) -> None:
        text = self._decoder.decode(data) if isinstance(data, bytes) else data
        with self._lock:
            self._raw += text
            self._lock.notify_all()

    def _ended(self) -> None:
        with self._lock:
            self._eof = True
            self._lock.notify_all()


def spawn(
    argv: Sequence[str],
    *,
    env: Mapping[str, str] | None = None,
    cwd: str | os.PathLike[str] | None = None,
    cols: int = 80,
    rows: int = 24,
) -> Terminal:
    return Terminal(
        argv, os.environ if env is None else env, os.fspath(cwd or os.getcwd()), cols, rows
    )


class _Unix:
    def __init__(self, terminal: Terminal) -> None:
        self._terminal = terminal

    def start(self, argv: list[str], env: dict[str, str], cwd: str, cols: int, rows: int) -> None:
        import pty

        self._master, slave = pty.openpty()
        self._set_size(slave, cols, rows)

        def controlling_terminal() -> None:
            import fcntl
            import termios

            fcntl.ioctl(0, termios.TIOCSCTTY, 0)

        self._process = subprocess.Popen(
            argv,
            stdin=slave,
            stdout=slave,
            stderr=slave,
            cwd=cwd,
            env=env,
            start_new_session=True,
            preexec_fn=controlling_terminal,  # noqa: PLW1509 - runs after setsid, before exec
            close_fds=True,
        )
        os.close(slave)
        self.pid = self._process.pid

    @staticmethod
    def _set_size(fd: int, cols: int, rows: int) -> None:
        import fcntl
        import struct
        import termios

        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))

    def read_loop(self) -> None:
        while True:
            try:
                data = os.read(self._master, 65536)
            except OSError:
                break
            if not data:
                break
            self._terminal._received(data)
        self._terminal._ended()

    def write(self, text: str) -> None:
        os.write(self._master, text.encode())

    def resize(self, cols: int, rows: int) -> None:
        self._set_size(self._master, cols, rows)

    def alive(self) -> bool:
        return self._process.poll() is None

    def exit_code(self) -> int:
        return self._process.wait()

    def close(self) -> None:
        import signal

        if self.alive():
            for sig, grace in ((signal.SIGHUP, 2.0), (signal.SIGKILL, 2.0)):
                try:
                    os.killpg(self._process.pid, sig)
                except ProcessLookupError:
                    break
                try:
                    self._process.wait(grace)
                    break
                except subprocess.TimeoutExpired:
                    continue
        with contextlib.suppress(OSError):
            os.close(self._master)


class _Windows:
    def __init__(self, terminal: Terminal) -> None:
        self._terminal = terminal

    def start(self, argv: list[str], env: dict[str, str], cwd: str, cols: int, rows: int) -> None:
        from winpty import Backend, PtyProcess  # pyright: ignore[reportMissingImports]

        self._process = PtyProcess.spawn(
            argv, cwd=cwd, env=env, dimensions=(rows, cols), backend=int(Backend.ConPTY)
        )
        self.pid = int(self._process.pid)

    def read_loop(self) -> None:
        while True:
            try:
                data = self._process.read(65536)
            except (EOFError, OSError):
                break
            if data:
                self._terminal._received(data)
        self._terminal._ended()

    def write(self, text: str) -> None:
        self._process.write(text)

    def resize(self, cols: int, rows: int) -> None:
        self._process.setwinsize(rows, cols)

    def alive(self) -> bool:
        return bool(self._process.isalive())

    def exit_code(self) -> int:
        return int(self._process.exitstatus)

    def close(self) -> None:
        if self.alive():
            self._process.terminate(force=True)
        with contextlib.suppress(Exception):  # best-effort release of the ConPTY
            self._process.close(force=True)
