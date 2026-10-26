"""OS observations of permissions, process identity and terminal ownership."""
import ctypes
import datetime as dt
import json
import os
from pathlib import Path
import signal
import stat
import subprocess
import sys

from harness import DEFAULT_TIMEOUT, pid_alive, wait_until

WINDOWS = sys.platform == "win32"
TRACKED_PIDS = set()

if WINDOWS:
    from ctypes import wintypes as w
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    advapi = ctypes.WinDLL("advapi32", use_last_error=True)
    kernel.OpenProcess.argtypes = [w.DWORD, w.BOOL, w.DWORD]
    kernel.OpenProcess.restype = w.HANDLE
    kernel.CloseHandle.argtypes = [w.HANDLE]
    kernel.TerminateProcess.argtypes = [w.HANDLE, w.UINT]
    kernel.QueryFullProcessImageNameW.argtypes = [w.HANDLE, w.DWORD, w.LPWSTR, ctypes.POINTER(w.DWORD)]
    kernel.GetProcessTimes.argtypes = [w.HANDLE] + [ctypes.POINTER(w.FILETIME)] * 4
    kernel.LocalFree.argtypes = [ctypes.c_void_p]
    kernel.CreateToolhelp32Snapshot.argtypes = [w.DWORD, w.DWORD]
    kernel.CreateToolhelp32Snapshot.restype = w.HANDLE
    kernel.FreeConsole.argtypes = []
    kernel.AttachConsole.argtypes = [w.DWORD]
    kernel.GetConsoleProcessList.argtypes = [ctypes.POINTER(w.DWORD), w.DWORD]
    kernel.GetConsoleProcessList.restype = w.DWORD

    class ProcessEntry(ctypes.Structure):
        _fields_ = [("size", w.DWORD), ("usage", w.DWORD), ("pid", w.DWORD),
                    ("heap", ctypes.c_size_t), ("module", w.DWORD), ("threads", w.DWORD),
                    ("ppid", w.DWORD), ("priority", w.LONG), ("flags", w.DWORD),
                    ("exe", w.WCHAR * 260)]

    for name in ("Process32FirstW", "Process32NextW"):
        getattr(kernel, name).argtypes = [w.HANDLE, ctypes.POINTER(ProcessEntry)]
    kernel.CreateFileW.argtypes = [w.LPCWSTR, w.DWORD, w.DWORD, ctypes.c_void_p, w.DWORD, w.DWORD, w.HANDLE]
    kernel.CreateFileW.restype = w.HANDLE
    advapi.GetSecurityInfo.argtypes = [w.HANDLE, ctypes.c_int, w.DWORD] + [ctypes.POINTER(ctypes.c_void_p)] * 5
    advapi.GetSecurityInfo.restype = w.DWORD
    READ_CONTROL = 0x00020000
    FILE_SHARE_ALL = 0x00000007
    OPEN_EXISTING = 3
    FILE_FLAG_OPEN_REPARSE_POINT = 0x00200000
    FILE_FLAG_BACKUP_SEMANTICS = 0x02000000
    advapi.ConvertSidToStringSidW.argtypes = [ctypes.c_void_p, ctypes.POINTER(w.LPWSTR)]
    advapi.ConvertStringSecurityDescriptorToSecurityDescriptorW.argtypes = [w.LPCWSTR, w.DWORD, ctypes.POINTER(ctypes.c_void_p), ctypes.POINTER(w.ULONG)]
    advapi.SetFileSecurityW.argtypes = [w.LPCWSTR, w.DWORD, ctypes.c_void_p]
    advapi.GetAce.argtypes = [ctypes.c_void_p, w.DWORD, ctypes.POINTER(ctypes.c_void_p)]


def checked(ok):
    if not ok:
        raise ctypes.WinError(ctypes.get_last_error())
    return ok


def sid_text(sid):
    text = w.LPWSTR()
    checked(advapi.ConvertSidToStringSidW(sid, ctypes.byref(text)))
    try:
        return text.value
    finally:
        kernel.LocalFree(text)


def security(path):
    owner, group, dacl, sacl, descriptor = [ctypes.c_void_p() for _ in range(5)]
    # Inspect AF_UNIX reparse points themselves, not their nonexistent targets.
    handle = kernel.CreateFileW(str(path), READ_CONTROL, FILE_SHARE_ALL, None, OPEN_EXISTING,
                                FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS, None)
    if handle == ctypes.c_void_p(-1).value:
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        code = advapi.GetSecurityInfo(handle, 1, 5, *[ctypes.byref(x) for x in (owner, group, dacl, sacl, descriptor)])
        if code:
            raise ctypes.WinError(code)
        owner_sid = sid_text(owner)
        assert dacl.value, f"{path}: null DACL grants public access"
        count = ctypes.c_ushort.from_address(dacl.value + 4).value
        aces = []
        for index in range(count):
            ace = ctypes.c_void_p()
            checked(advapi.GetAce(dacl, index, ctypes.byref(ace)))
            kind = ctypes.c_ubyte.from_address(ace.value).value
            assert kind in (0, 1), f"{path}: unexpected ACL entry type {kind}"
            aces.append((kind, sid_text(ace.value + 8)))
        return owner_sid, aces
    finally:
        if descriptor.value:
            kernel.LocalFree(descriptor)
        kernel.CloseHandle(handle)


def make_private(path):
    path = Path(path)
    if not WINDOWS:
        path.chmod(0o700 if path.is_dir() else 0o600)
        return
    owner, _ = security(path)
    descriptor = ctypes.c_void_p()
    checked(advapi.ConvertStringSecurityDescriptorToSecurityDescriptorW(
        f"D:P(A;;FA;;;{owner})", 1, ctypes.byref(descriptor), None))
    try:
        checked(advapi.SetFileSecurityW(str(path), 0x80000004, descriptor))
    finally:
        kernel.LocalFree(descriptor)


def assert_private(path, mode):
    path = Path(path)
    if not WINDOWS:
        info = path.stat()
        assert info.st_uid == os.getuid(), f"{path}: not owned by session user"
        assert stat.S_IMODE(info.st_mode) == mode, f"{path}: permissions must be {mode:04o} from first visibility"
    else:
        owner, aces = security(path)
        current = subprocess.run(["whoami", "/user", "/fo", "csv", "/nh"], capture_output=True, text=True, timeout=DEFAULT_TIMEOUT, check=True)
        import csv
        user_sid = next(csv.reader([current.stdout.strip()]))[1]
        assert owner == user_sid, f"{path}: not owned by session user"
        assert aces and all(kind == 0 and sid == user_sid for kind, sid in aces), f"{path}: ACL must grant access only to user SID"


def processes():
    result = {}
    if WINDOWS:
        snapshot = kernel.CreateToolhelp32Snapshot(2, 0)
        if snapshot == ctypes.c_void_p(-1).value:
            raise ctypes.WinError()
        try:
            entry = ProcessEntry()
            entry.size = ctypes.sizeof(entry)
            more = kernel.Process32FirstW(snapshot, ctypes.byref(entry))
            while more:
                path = entry.exe
                handle = kernel.OpenProcess(0x1000, False, entry.pid)
                if handle:
                    try:
                        buf, size = ctypes.create_unicode_buffer(32768), w.DWORD(32768)
                        if kernel.QueryFullProcessImageNameW(handle, 0, buf, ctypes.byref(size)):
                            path = buf.value
                    finally:
                        kernel.CloseHandle(handle)
                result[entry.pid] = {"ppid": entry.ppid, "exe": path}
                more = kernel.Process32NextW(snapshot, ctypes.byref(entry))
        finally:
            kernel.CloseHandle(snapshot)
    elif sys.platform == "linux":
        for path in Path("/proc").iterdir():
            if not path.name.isdigit():
                continue
            try:
                fields = (path / "stat").read_text().rsplit(")", 1)[1].split()
                exe = os.readlink(path / "exe")
                result[int(path.name)] = {"ppid": int(fields[1]), "tty": int(fields[4]), "exe": exe, "ticks": int(fields[19])}
            except (FileNotFoundError, ProcessLookupError, PermissionError):
                continue
    else:
        output = subprocess.run(["ps", "-axo", "pid=,ppid=,tty=,comm="], env={**os.environ, "LC_ALL": "en_US.UTF-8"}, capture_output=True, text=True, encoding="utf-8", check=True, timeout=DEFAULT_TIMEOUT).stdout
        for line in output.splitlines():
            pid, parent, tty, exe = line.strip().split(None, 3)
            result[int(pid)] = {"ppid": int(parent), "tty": tty, "exe": exe}
    return result


def descendants(pid, table=None):
    table = processes() if table is None else table
    found = {pid}
    while True:
        more = {p for p, info in table.items() if info["ppid"] in found}
        if more <= found:
            return found - {pid}
        found |= more


def terminate(pid):
    if not pid_alive(pid):
        return
    if WINDOWS:
        handle = kernel.OpenProcess(1, False, pid)
        if handle:
            try:
                checked(kernel.TerminateProcess(handle, 99))
            finally:
                kernel.CloseHandle(handle)
    else:
        try:
            os.kill(pid, signal.SIGKILL)
        except ProcessLookupError:
            pass


def process_start(pid):
    if WINDOWS:
        handle = checked(kernel.OpenProcess(0x1000, False, pid))
        times = [w.FILETIME() for _ in range(4)]
        try:
            checked(kernel.GetProcessTimes(handle, *[ctypes.byref(t) for t in times]))
            return ((times[0].dwHighDateTime << 32) + times[0].dwLowDateTime) / 10000000 - 11644473600
        finally:
            kernel.CloseHandle(handle)
    if sys.platform == "linux":
        boot = next(int(line.split()[1]) for line in Path("/proc/stat").read_text().splitlines() if line.startswith("btime "))
        return boot + processes()[pid]["ticks"] / os.sysconf("SC_CLK_TCK")
    output = subprocess.run(["ps", "-p", str(pid), "-o", "lstart="], env={**os.environ, "LC_ALL": "en_US.UTF-8"}, capture_output=True, text=True, encoding="utf-8", check=True, timeout=DEFAULT_TIMEOUT).stdout.strip()
    return dt.datetime.strptime(output, "%a %b %d %H:%M:%S %Y").timestamp()


def _attached_console_processes(pid):
    checked(kernel.FreeConsole())
    if not kernel.AttachConsole(pid):
        code = ctypes.get_last_error()
        if code == 6:  # ERROR_INVALID_HANDLE: the target has no console.
            return []
        raise ctypes.WinError(code)
    try:
        capacity = 256
        while True:
            ids = (w.DWORD * capacity)()
            count = checked(kernel.GetConsoleProcessList(ids, capacity))
            if count <= capacity:
                return list(ids[:count])
            capacity = count
    finally:
        checked(kernel.FreeConsole())


def console_processes(pid):
    # Attach in a short-lived observer so pytest's own console is never changed.
    result = subprocess.run(
        [sys.executable, "-c",
         "import json, sys; sys.path.insert(0, sys.argv[1]); "
         "from native import _attached_console_processes; "
         "print(json.dumps(_attached_console_processes(int(sys.argv[2]))))",
         str(Path(__file__).resolve().parent), str(pid)],
        capture_output=True, text=True, check=True, timeout=DEFAULT_TIMEOUT)
    return json.loads(result.stdout)


def assert_detached(pid):
    info = processes()[pid]
    assert "climon2-daemon" in Path(info["exe"]).name, "daemon_pid must identify the actual session daemon"
    if not WINDOWS:
        assert info["tty"] in (0, "??", "?", "-"), "session daemon must have no controlling terminal"
