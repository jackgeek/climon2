"""Native AF_UNIX transport, including Winsock where Python omits AF_UNIX."""
import ctypes
import os
import socket
import struct
import time


if os.name == "nt":
    from ctypes import wintypes

    ws = ctypes.WinDLL("ws2_32", use_last_error=True)
    SOCKET = ctypes.c_size_t
    INVALID = SOCKET(-1).value
    ws.WSAStartup.argtypes = [wintypes.WORD, ctypes.c_void_p]
    ws.WSAGetLastError.restype = ctypes.c_int
    ws.socket.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_int]
    ws.socket.restype = SOCKET
    ws.accept.argtypes = [SOCKET, ctypes.c_void_p, ctypes.c_void_p]
    ws.accept.restype = SOCKET
    for name in ("bind", "connect"):
        getattr(ws, name).argtypes = [SOCKET, ctypes.c_void_p, ctypes.c_int]
    ws.listen.argtypes = [SOCKET, ctypes.c_int]
    ws.closesocket.argtypes = [SOCKET]
    ws.ioctlsocket.argtypes = [SOCKET, ctypes.c_long, ctypes.POINTER(wintypes.ULONG)]
    for name in ("send", "recv"):
        getattr(ws, name).argtypes = [SOCKET, ctypes.c_void_p, ctypes.c_int, ctypes.c_int]
    ws.getsockopt.argtypes = [SOCKET, ctypes.c_int, ctypes.c_int, ctypes.c_void_p, ctypes.POINTER(ctypes.c_int)]

    class PollFD(ctypes.Structure):
        _fields_ = [("fd", SOCKET), ("events", ctypes.c_short), ("revents", ctypes.c_short)]

    ws.WSAPoll.argtypes = [ctypes.POINTER(PollFD), wintypes.ULONG, ctypes.c_int]
    ws.WSAIoctl.argtypes = [SOCKET, wintypes.DWORD, ctypes.c_void_p, wintypes.DWORD, ctypes.c_void_p, wintypes.DWORD, ctypes.POINTER(wintypes.DWORD), ctypes.c_void_p, ctypes.c_void_p]
    data = ctypes.create_string_buffer(512)
    if ws.WSAStartup(0x202, data):
        raise OSError("WSAStartup failed")


class LocalSocket:
    def __init__(self, handle=None):
        self.timeout = 2.0
        if os.name == "nt":
            self.handle = ws.socket(1, 1, 0) if handle is None else handle
            if self.handle == INVALID:
                raise OSError(ws.WSAGetLastError(), "AF_UNIX socket")
            flag = wintypes.ULONG(1)
            self._check(ws.ioctlsocket(self.handle, -2147195266, ctypes.byref(flag)))
        else:
            self.handle = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) if handle is None else handle
            self.handle.settimeout(self.timeout)

    def _check(self, result):
        if result == -1:
            raise OSError(ws.WSAGetLastError(), "Winsock AF_UNIX")
        return result

    def settimeout(self, value):
        self.timeout = max(0.001, value)
        if os.name != "nt":
            self.handle.settimeout(self.timeout)

    def _ready(self, write=False):
        fd = PollFD(self.handle, 0x10 if write else 0x100, 0)
        n = self._check(ws.WSAPoll(ctypes.byref(fd), 1, max(1, int(self.timeout * 1000))))
        if not n:
            raise TimeoutError("AF_UNIX deadline expired")

    def _address(self, path):
        raw = os.fsencode(path)
        if len(raw) >= 108:
            raise ValueError("test AF_UNIX path too long")
        return ctypes.create_string_buffer(struct.pack("H", 1) + raw + b"\0" * (108 - len(raw)))

    def bind(self, path):
        if os.name == "nt":
            address = self._address(path)
            self._check(ws.bind(self.handle, address, 110))
        else:
            self.handle.bind(str(path))

    def listen(self):
        if os.name == "nt":
            self._check(ws.listen(self.handle, 32))
        else:
            self.handle.listen(32)

    def connect(self, path):
        if os.name != "nt":
            self.handle.connect(str(path))
            return
        address = self._address(path)
        result = ws.connect(self.handle, address, 110)
        if result == -1:
            code = ws.WSAGetLastError()
            if code not in (10035, 10036):
                raise OSError(code, "AF_UNIX connect")
            self._ready(write=True)
            error = ctypes.c_int()
            size = ctypes.c_int(4)
            self._check(ws.getsockopt(self.handle, 0xffff, 0x1007, ctypes.byref(error), ctypes.byref(size)))
            if error.value:
                raise OSError(error.value, "AF_UNIX connect")

    def accept(self):
        if os.name != "nt":
            child, _ = self.handle.accept()
            return LocalSocket(child)
        self._ready()
        child = ws.accept(self.handle, None, None)
        if child == INVALID:
            raise OSError(ws.WSAGetLastError(), "AF_UNIX accept")
        return LocalSocket(child)

    def recv(self, size):
        if os.name != "nt":
            return self.handle.recv(size)
        self._ready()
        buf = ctypes.create_string_buffer(size)
        n = self._check(ws.recv(self.handle, buf, size, 0))
        return buf.raw[:n]

    def sendall(self, data):
        if os.name != "nt":
            self.handle.sendall(data)
            return
        deadline = time.monotonic() + self.timeout
        while data:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("AF_UNIX send deadline expired")
            old = self.timeout
            self.timeout = remaining
            try:
                self._ready(write=True)
                n = self._check(ws.send(self.handle, data, len(data), 0))
            finally:
                self.timeout = old
            if not n:
                raise ConnectionError("AF_UNIX send closed")
            data = data[n:]

    def peer_pid(self):
        if os.name != "nt":
            raise NotImplementedError("SIO_AF_UNIX_GETPEERPID is Windows-specific")
        pid, used = wintypes.DWORD(), wintypes.DWORD()
        self._check(ws.WSAIoctl(self.handle, 0x58000100, None, 0, ctypes.byref(pid), 4, ctypes.byref(used), None, None))
        assert used.value == 4 and pid.value > 0, "peer PID ioctl returned no verifiable identity"
        return pid.value

    def close(self):
        if self.handle is not None:
            if os.name == "nt":
                ws.closesocket(self.handle)
            else:
                self.handle.close()
            self.handle = None

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()


def receive_exact(sock, count, deadline):
    data = bytearray()
    while len(data) < count:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError("frame exceeded total deadline")
        sock.settimeout(remaining)
        chunk = sock.recv(count - len(data))
        if not chunk:
            raise EOFError("connection closed during frame")
        data.extend(chunk)
    return bytes(data)


def frame(payload):
    return struct.pack(">I", len(payload)) + payload


def receive_frame(sock, deadline):
    length = struct.unpack(">I", receive_exact(sock, 4, deadline))[0]
    assert 0 < length <= 1048576, "reply frame length outside protocol 1 limits"
    return receive_exact(sock, length, deadline)
