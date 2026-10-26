# Unix domain sockets on every platform, including Windows

Session daemons and clients communicate over AF_UNIX sockets on all platforms, rather than named pipes on Windows. AF_UNIX is available on Windows 10 1803+ and Server 2019+. One transport keeps the protocol and tests uniform.

Access is limited to the same user in two ways:
- **Runtime directory:** it must be user-only. Unix requires mode 0700; Windows requires a user-SID-only ACL. A more permissive directory is refused.
- **Peer check:** on macOS the peer UID comes from `getpeereid`, on Linux from `SO_PEERCRED`. On Windows the factory gets the peer PID with `SIO_AF_UNIX_GETPEERPID`, then compares the token user SID of that process.

## Consequences

- Whether `SIO_AF_UNIX_GETPEERPID` is available on Windows Server 2025 is unverified. If it's missing, the feature pauses for a contract amendment instead of weakening the check.
- Socket paths must be short, so they go in a per-user runtime directory rather than climon2 home:
  - Unix: `$XDG_RUNTIME_DIR` or `/tmp/climon2-<uid>/`
  - Windows: `%LOCALAPPDATA%\climon2\run\` with hashed names
