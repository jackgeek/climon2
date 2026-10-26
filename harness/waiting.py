"""Readiness waits on observable conditions, and process liveness."""

import os
import sys
import time
from collections.abc import Callable

from harness.terminal import DEFAULT_TIMEOUT


def wait_until(
    condition: Callable[[], object], message: str, timeout: float = DEFAULT_TIMEOUT
) -> object:
    """Poll `condition` until it returns something truthy; fail with `message` at the deadline."""
    deadline = time.monotonic() + timeout
    while True:
        result = condition()
        if result:
            return result
        if time.monotonic() > deadline:
            raise AssertionError(f"timed out after {timeout}s: {message}")
        time.sleep(0.05)


def pid_alive(pid: int) -> bool:
    """Whether a process with this id exists (and, on Unix, is not a zombie we can see)."""
    if sys.platform == "win32":
        import ctypes

        kernel32 = ctypes.windll.kernel32  # pyright: ignore[reportAttributeAccessIssue]
        handle = kernel32.OpenProcess(0x1000, False, pid)  # PROCESS_QUERY_LIMITED_INFORMATION
        if not handle:
            return False
        code = ctypes.c_ulong()
        try:
            kernel32.GetExitCodeProcess(handle, ctypes.byref(code))
            return code.value == 259  # STILL_ACTIVE
        finally:
            kernel32.CloseHandle(handle)
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    try:
        reaped, _ = os.waitpid(pid, os.WNOHANG)
    except ChildProcessError:
        return True
    return reaped == 0
