# Windows AF_UNIX transport and non-`poll` I/O

Research for [#208](https://github.com/jackgeek/climon2/issues/208), a child of
the Windows support map [#205](https://github.com/jackgeek/climon2/issues/205).
It records facts and options; it makes no policy decisions.

Scope: Windows 11 on x86_64 MSVC only. Windows 10 is not supported. Repo state
read at `fd25a71`. Crate versions are the ones in `Cargo.lock` (socket2 0.6.5,
polling 3.11.0, portable-pty 0.9.0, mio 1.2.4, tokio 1.53.1, windows-sys 0.61.2)
or the latest release where the lockfile has none (uds_windows 1.2.1,
async-io 2.6.0). The toolchain is pinned to Rust 1.96.0 (`rust-toolchain.toml`).

Confidence labels:

- **Documented**: stated by the owning vendor's docs or the crate's source.
- **Reported**: comes from third-party evidence, not from vendor docs. Confirm
  it in Windows CI before relying on it.
- **Inference**: a deduction from documented facts. It has not been tested.

## Summary

1. **AF_UNIX on Windows is stream-only.** It has filesystem paths, abstract
   and unnamed addresses, no ancillary data, no `socketpair` and no
   `SO_PEERCRED` [1]. `sun_path` is 108 bytes of UTF-8 [2][3]. socket2 0.6.5
   and std both reject a path of 108 bytes or more, so the usable maximum is
   107 bytes plus the terminating NUL [4][5]. The current Unix budget is 103
   bytes.
2. **The socket file is an NTFS reparse point.** Its tag is
   `IO_REPARSE_TAG_AF_UNIX` (`0x80000023`) [6][7]. std's `symlink_metadata`
   opens it without following it. std reports it as a regular *file*, not a
   symlink, because the tag lacks the name-surrogate bit [8]. A socket check
   must read the reparse tag through a Win32 call; std has no stable accessor
   for it. Windows never removes the file for you, so call `DeleteFile` before
   re-binding the same path [1].
3. **Stale-socket errors.** A leftover socket file with no listener is
   reported to give `WSAECONNREFUSED` (10061) [9]. std maps that code to
   `ErrorKind::ConnectionRefused` [10]. The error for a missing path is not
   documented anywhere I could find. Third-party reports disagree: one says it
   is a not-found code [9], another says it is 10061 [11]. A check on
   `ErrorKind::{NotFound, ConnectionRefused}` covers both cases on every OS.
   On Unix it means exactly what `ENOENT | ECONNREFUSED` means today [10][12].
4. **Peer identity.** Windows has no `SO_PEERCRED`. It does have the ioctl
   `SIO_AF_UNIX_GETPEERPID`, which is defined in `afunix.h` and windows-sys
   [3][13]. That PID could feed a process-token user-SID comparison. Learn
   does not document the ioctl, so treat it as **reported**.
5. **Directory DACL alone is weaker than Unix.** `bind` needs write access to
   the directory, and `connect` needs write access to the socket file [1]. A
   protected owner-only DACL therefore keeps other *standard* users out.
   Holders of `SeRestorePrivilege` (Administrators) are granted write access
   regardless of the ACL [14]. Today's Unix `SO_PEERCRED` check rejects even
   root, because root's UID differs from the user's. The DACL-only design in
   ADR 0003 does not keep that property; a PID-to-SID peer check would.
6. **Multiplexing.** `WSAPoll` accepts sockets only [15]. ConPTY channels must
   be synchronous handles, and Microsoft recommends a separate thread per
   channel [16][17]. Anonymous pipes (what portable-pty creates) do not support
   overlapped I/O [18][19]. Console input handles are waitable, but pipes are
   not [20]. No single Windows wait primitive covers sockets, ConPTY pipes and
   console output at once. Every viable design therefore uses at least one
   helper thread for each ConPTY pipe direction.
7. **CI cannot exercise Windows 11 itself.** GitHub's x64 Windows runners are
   Windows Server 2022 (build 20348) and Server 2025 (build 26100). There is
   no x64 Windows 11 hosted image [21][22][23]. The runners run as an
   administrator with UAC disabled [21]. See
   [Windows 11 vs. GitHub runners](#windows-11-vs-github-hosted-runners).

## 1. AF_UNIX on Windows 11

### 1.1 Feature set

From Microsoft's AF_UNIX announcement [1]:

| Feature | Windows | Today's Unix use in climon2 |
| --- | --- | --- |
| `SOCK_STREAM` | Supported ("support only exists for the stream (SOCK_STREAM) socket type") | Used everywhere |
| `SOCK_DGRAM`, `SOCK_SEQPACKET` | Not supported | Not used |
| Pathname addresses | `sun_path` is "a Win32 UTF-8 file system path" | Used |
| Abstract addresses | "can also accept abstract addresses" | Not used |
| Unnamed sockets | Supported | Not used |
| Autobind | Not supported | Not used |
| Ancillary data (fd passing, `SCM_CREDENTIALS`) | "There is no support for ancillary data" | Not used |
| `socketpair` | "not supported in Winsock 2.0" | Used in Unix unit tests (`UnixStream::pair`) |
| `SO_PEERCRED` | Not mentioned in [1]. No such option in the Windows headers [3]. | `peer/linux.rs`, `peer/macos.rs` |

Without `socketpair`, Windows tests need a listener, a connect and an accept
on a temporary path. uds_windows emulates `pair()` exactly that way [24].

`afunix.h` defines `UNIX_PATH_MAX` as 108 and `SOCKADDR_UN { sun_family;
sun_path[UNIX_PATH_MAX] }`. It also defines three vendor ioctls:
`SIO_AF_UNIX_GETPEERPID` (`_WSAIOR(IOC_VENDOR, 256)`),
`SIO_AF_UNIX_SETBINDPARENTPATH` (257) and `SIO_AF_UNIX_SETCONNPARENTPATH` (258)
[3]. windows-sys 0.61.2 exports the same constants:
`SIO_AF_UNIX_GETPEERPID = 1476395264`, `SOCKADDR_UN.sun_path: [i8; 108]` [13].
I found no Learn page for any of the three ioctls. The two `*PARENTPATH`
ioctls look as if they could bind or connect relative to a directory and so
ease the path limit, but their semantics are undocumented (**reported** /
unknown).

### 1.2 Path limit

- Windows: 108 bytes of UTF-8 including the NUL [1][3].
- socket2 0.6.5 `unix_sockaddr` on Windows requires a UTF-8 `Path`
  (`to_str()`, else `InvalidInput "path must be valid UTF-8"`). It rejects
  `bytes.len() >= sun_path.len()`, so the maximum is 107 bytes [4].
- The unstable std `std::os::windows::net` `sockaddr_un` applies the same
  `>= sun_path.len()` rejection [5].
- climon2 today: `runtime::bind` rejects paths over 103 bytes, and
  `RuntimeLayout::socket_path(id, 103)` applies the same bound
  (`crates/climon2-platform/src/runtime.rs`,
  `crates/climon2-core/src/layout.rs`).

Path budget (**inference**). The layout is
`<root>\climon2-<uid>-<hash8>\<36-char GUID>.sock`; `.exec` has the same
length. Take `%TEMP%` = `C:\Users\<U>\AppData\Local\Temp`, which is
29 + U bytes:

- With the Unix `climon2-<uid>-<hash8>` naming and a 10-digit number in place
  of the uid, the total is 29 + U + 1 + 27 + 1 + 41 = 99 + U bytes. That fits
  107 only while the profile folder name is 8 bytes or fewer.
- With a `climon2-<hash8>` naming that drops the uid, the total is 88 + U
  bytes, which allows a profile folder name of up to 19 bytes.

Non-ASCII user names take 2–3 UTF-8 bytes per character. `%TEMP%` can also be
redirected. Windows has no numeric uid: `RuntimeLayout::new` takes a `u32`,
and Windows identity is a SID. How the uid component is named on Windows is a
policy decision.

### 1.3 The socket file on disk

- `bind` creates a socket file, and the file is "a custom NTFS reparse point"
  [1]. Its tag is `IO_REPARSE_TAG_AF_UNIX = 0x80000023`, "used to represent a
  UNIX domain socket", meaningful only server-side, with no defined structure
  [6]. windows-sys exports `IO_REPARSE_TAG_AF_UNIX = 2147483683` [7].
- std 1.96 `fs::symlink_metadata` on Windows opens the path with
  `FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT`. It then reads
  `FileAttributeTagInfo` to get the reparse tag [8]. Its `FileType` counts as
  a symlink only when the reparse point's tag has the name-surrogate bit
  `0x20000000`. `0x80000023` lacks that bit, so an AF_UNIX socket gives
  `is_file() == true`, `is_symlink() == false` [8].
  `std::os::windows::fs::MetadataExt::file_attributes()` is stable and
  exposes `FILE_ATTRIBUTE_REPARSE_POINT`. std does not expose the tag [8].
  Today's `validate_stale_socket` uses Unix `FileTypeExt::is_socket()`.
  Reproducing it on Windows needs a raw
  `GetFileInformationByHandleEx(FileAttributeTagInfo)` call on a handle
  opened with `FILE_FLAG_OPEN_REPARSE_POINT`, or
  `FindFirstFileW` → `dwReserved0` (**inference** from [8]).
- Wine and Cygwin both report that opening the socket file *without*
  `FILE_OPEN_REPARSE_POINT` fails with `STATUS_IO_REPARSE_TAG_NOT_HANDLED`
  [25][26] (**reported**). In practice the reparse-point flag is mandatory
  for any metadata or delete operation.
- Deletion: "DeleteFile (or any other file delete API) should be used to
  delete the socket file prior to calling bind with the same path" [1].
  std's own Windows tests call `remove_file` before `bind`. They also assert
  that a second `bind` on a live socket path fails [27]. `fs::remove_file` on
  Windows deletes a reparse point itself, not its target (**inference**).
- After the process dies: nothing in [1] says the file is removed, and the
  instruction to `DeleteFile` before re-binding implies it is not. The file
  stays until someone deletes it, as on Unix. This matches climon2's existing
  stale-metadata cleanup model.
- Whether the socket file can be deleted *while the listener is alive*, as
  `unlink` allows on Unix, is undocumented. **Needs CI evidence.**

### 1.4 Error codes for missing and refused sockets

| Situation | Windows evidence | std `ErrorKind` |
| --- | --- | --- |
| Socket file exists, no listener (stale) | `WSAECONNREFUSED` 10061 ("Connection refused … no server application running") [28]. Reported for stale AF_UNIX files on Windows 11 [9]. | `ConnectionRefused` [10] |
| Path does not exist | Not documented. One report says it surfaces as a not-exist error (Go `fs.ErrNotExist`) [9]. Another says 10061 [11]. | `NotFound` if the code is `ERROR_FILE_NOT_FOUND`/`ERROR_PATH_NOT_FOUND` [10]. `ConnectionRefused` if it is 10061. |
| Nonblocking connect pending | `WSAEWOULDBLOCK` 10035 [28]. socket2 treats it as "in progress" and waits with `WSAPoll` [4]. | `WouldBlock` |

The current stale check in `crates/climon2-cli/src/list.rs` matches
`raw_os_error()` against `libc::ENOENT | libc::ECONNREFUSED`. std's Unix
decoder maps `ENOENT`→`NotFound` and `ECONNREFUSED`→`ConnectionRefused` [12].
A shared `error.kind()` check would therefore behave identically on Unix and
cover both candidate Windows codes. Option B keeps the libc match in a Unix
`cfg` body and adds a Windows body.

Note (**inference**): `ConnectionRefused` alone does not prove that the
daemon is dead. climon2 already handles this by requiring
`process::is_same_daemon == false` before cleanup, and that guard is
platform-neutral in shape.

### 1.5 Library support

| Library | Windows AF_UNIX? | Notes |
| --- | --- | --- |
| **std** (Rust 1.96) | **Unstable only.** `std::os::windows::net::{UnixListener, UnixStream}` sit behind `#![feature(windows_unix_domain_sockets)]`, tracking issue 150487 [5]. | No `pair()`. Has `set_nonblocking`, timeouts, `take_error`, `AsSocket`/`AsRawSocket`/`FromRawSocket` [5]. Can't be used on the pinned stable toolchain. |
| **socket2 0.6.5** | **Yes.** `Domain::UNIX` maps to `AF_UNIX`, and `SockAddr::unix` builds a `SOCKADDR_UN` on Windows [4]. | Already a dependency of `climon2-platform`, and `connection::connect` already uses `SockAddr::unix` + `connect_timeout`. On Windows, `connect_timeout` waits with `WSAPoll` [4]. `socket2::Socket` converts to and from `OwnedSocket`, but there is no std `UnixStream` to convert into. A Windows `SocketStream` type would be `socket2::Socket` or a newtype over it. Note: socket2 0.6.5's Windows target depends on `windows-sys >=0.60, <0.62` [4]. |
| **uds_windows 1.2.1** | Yes. `UnixListener`/`UnixStream` with std-like APIs, and a `pair()` emulated through a temp-dir listener [24]. | Maintained (published 2026-03-14), with heavy reverse dependencies (zbus, async-io, smol) [29]. Duplicates what socket2 already provides. |
| **mio 1.2.4** | **No.** `UnixStream`/`UnixListener`/`UnixDatagram` are `#[cfg(unix)]` [30]. | Windows offers TCP, UDP and `NamedPipe` only. |
| **tokio 1.53** | **No.** `cfg_net_unix!` is `#[cfg(all(unix, feature = "net"))]`. Windows has only `net::windows::named_pipe` [31]. | |
| **polling 3.11.0** | **Yes, for any socket** through `IOCTL_AFD_POLL` on an IOCP [32]. async-io's `windows-uds` example registers uds_windows AF_UNIX sockets with it [33]. | Also waits on waitable handles (console input, events, processes) through `PollerIocpExt::add_waitable`, and accepts posted completion packets [32]. Already in `Cargo.lock` as a transitive dev-dependency (through `alacritty_terminal`). It relies on undocumented ntdll AFD calls, which the crate itself says are "unlikely to be broken without plenty of advanced warning" [32]. |
| **filedescriptor 0.8.3** (through portable-pty) | `poll()` on Windows is `WSAPoll`, so sockets only. Its `socketpair` on Windows is an AF_INET loopback pair, not AF_UNIX [34]. | |

## 2. Securing the runtime directory under `%TEMP%`

Facts:

- A new object's DACL is the creator's explicit DACL, merged with inheritable
  ACEs unless `SE_DACL_PROTECTED` is set. Otherwise it comes from inherited
  ACEs, and otherwise from the token's default DACL [35]. A directory created
  with an explicit, protected DACL therefore stops inheritance from `%TEMP%`.
  Files created inside it, including socket files, inherit its inheritable
  ACEs.
- AF_UNIX access control is file access control. Creating the socket needs
  write permission on the directory, and connecting needs write permission on
  the socket file. "The same level of security is available and enforced"
  [1].
- On NTFS, file and directory owners and DACLs are read through
  `GetNamedSecurityInfo`/`GetSecurityInfo` [36]. That is the Windows
  counterpart of today's `check_directory` (`metadata.uid()` and
  `mode & 0o077`).
- Privileged bypasses [37]:
  - `SeBackupPrivilege` grants read access "regardless of the ACL".
  - `SeRestorePrivilege` grants "all write access control to any file,
    regardless of the ACL", including `FILE_GENERIC_WRITE`, `WRITE_DAC`,
    `WRITE_OWNER` and `DELETE`.
  - `SeTakeOwnershipPrivilege` allows taking ownership without discretionary
    access.
  - `SeDebugPrivilege` ignores process DACLs.

  These privileges are held by Administrators. They are effective when the
  token is elevated, or on runners where UAC is disabled.

Is "owner SID check + owner-only protected DACL" sufficient?

- Against other standard users: yes. They lack write access to the directory
  and the socket file, so `bind` squatting and `connect` both fail [1][35].
- Against administrators and SYSTEM: no. `SeRestorePrivilege` gives write
  access, and therefore `connect`, regardless of the DACL [37]. Unix root can
  `connect` too, but climon2's Unix `SO_PEERCRED` UID check then rejects the
  root peer. ADR 0003's Windows note (peer check is a no-op) is therefore not
  equivalent to Unix here. Whether that matters is a policy question.
- Optional per-connection check (**reported** ioctl, documented follow-on
  APIs):
  1. `WSAIoctl(SIO_AF_UNIX_GETPEERPID)` [3][13].
  2. `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)`.
  3. `OpenProcessToken`.
  4. `GetTokenInformation(TokenUser)`.
  5. Compare the SID with the current user's.

  Wine's bug tracker describes the ioctl as the Windows analogue of
  `SO_PEERCRED`, used by GLib and D-Bus [38]. Two caveats (**inference**):
  - The PID could refer to a process that has since exited, so the PID could
    be reused. Climon2's daemon-identity work faces the same class of issue.
  - A same-user process passes a SID comparison whether or not it is
    elevated. Policy decides whether to also compare elevation or integrity
    level.

## 3. Multiplexing without `poll`

### 3.1 What polls today

| Site | Handles polled | Windows counterpart handle |
| --- | --- | --- |
| `climon2-platform/src/connection.rs` `wait` | One socket, `POLLIN` or `POLLOUT`, with timeout | AF_UNIX socket |
| `climon2-client/src/transport.rs` `poll`/`Transport::wait` | One socket, 25 ms slices, checks the signal flag between slices | AF_UNIX socket |
| `climon2-client/src/bridge.rs` `Bridge::poll` / `probe_hangup` | Socket + terminal input + stdout (`POLLOUT`), ≤20 ms ticks, plus `O_NONBLOCK` on the tty (`Nonblocking` guard) | Socket + console input handle (or a pipe if redirected) + console output handle (or a pipe) |
| `climon2-daemon/src/pty.rs` `wait_readable` | PTY master `POLLIN`, `TICK` = 2 ms. The daemon loop is otherwise nonblocking and tick-driven (`session.rs`). | ConPTY output pipe (anonymous, from portable-pty `filedescriptor::Pipe::new()` = `CreatePipe`) [19][34] |
| `climon2-daemon/src/pty.rs` `ExecListener::accept` | Nonblocking accept with 2 ms sleeps | Socket |
| `climon2-cli/src/list.rs` `exchange_hello` | Nonblocking socket under a deadline | Socket |

### 3.2 Windows primitives

- **`WSAPoll`**: `poll` semantics, sockets only. Negative `fd` entries are
  ignored (they report `POLLNVAL`). `POLLHUP` means a stream connection was
  disconnected or aborted. `POLLPRI` makes the call fail. Since Windows 10
  2004, a failed TCP connect reports `POLLHUP | POLLERR | POLLWRNORM` [15].
  It maps directly onto `connection::wait`, `Transport::wait` and the
  socket-only loops.
- **`WSAEventSelect` + `WaitForMultipleObjects`**: this ties a socket to an
  event object and *forces the socket nonblocking*. `FD_READ`/`FD_ACCEPT`
  are level-triggered and re-enabled by `recv`/`accept`. `FD_WRITE` is
  recorded only on connect/accept and again after a send hits
  `WSAEWOULDBLOCK`. `FD_CLOSE` is posted after all data has been read. One
  event per socket. Reset through `WSAEnumNetworkEvents` [39].
  `WaitForMultipleObjects` waits on ≤ `MAXIMUM_WAIT_OBJECTS` handles. Allowed
  types are console input, events, processes, threads, waitable timers,
  mutexes, semaphores, change notifications and memory notifications. Pipes
  and files are not on the list [20].
- **Console input** is waitable [20]. The input buffer carries key, mouse,
  window-buffer-size, focus and menu records, so a signalled handle does not
  imply that `ReadFile` has bytes to return [40] (**inference**: a read after
  a non-key record can block). Console *output* has no readiness signal.
- **Anonymous pipes**: no overlapped I/O ("lpOverlapped … is ignored").
  `WriteFile` blocks until all bytes are written. `ReadFile` returns when data
  arrives or when all write handles close [18]. `PeekNamedPipe` works on the
  read end of an anonymous pipe and returns the bytes available without
  removing them. It returns immediately in a single-threaded program, but "can
  block … on a synchronous handle in a multi-threaded application" [41].
- **ConPTY**: `hInput`/`hOutput` are "restricted to synchronous I/O" [16].
  Microsoft "highly recommend[s] that each of the communication channels is
  serviced on a separate thread". `ClosePseudoConsole` may emit a final frame
  that must be drained, or it deadlocks; keep reader threads draining "until
  broken of their own accord" [17].
- **Cancellation**: `CancelSynchronousIo(thread)` cancels a blocking
  synchronous call on another thread, and `CancelIoEx` cancels overlapped
  I/O. Cancelled calls fail with `ERROR_OPERATION_ABORTED`. "There is no
  guarantee that underlying drivers correctly support cancellation", and
  synchronous cancellation is racy if the thread starts another call [42].
  Closing all write handles ends a blocked `ReadFile` on a pipe [18]. For
  ConPTY, `ClosePseudoConsole` plus closing our pipe ends breaks the channel
  [17].
- **Timeouts**: `WSAPoll` and `WaitForMultipleObjects` take millisecond
  timeouts [15][20]. Since Windows 8, wait timeouts exclude time in low-power
  states [20], whereas climon2 deadlines use `Instant`.

### 3.3 Options

All options keep the Unix bodies (`libc::poll`, `O_NONBLOCK`) unchanged in
`#[cfg(unix)]` files. They differ in how much Windows-only structure they
need.

**A. Thread-per-handle, with sockets polled by `WSAPoll`.**

- One blocking reader thread per non-socket source: the ConPTY output pipe in
  the daemon, and console or pipe stdin in the client. One writer thread per
  blocking sink: the ConPTY input pipe, and console stdout. Threads exchange
  bytes with the main loop through queues.
- The main loop stays a tick or poll loop over sockets only (`WSAPoll`). It
  checks queues each tick. The daemon already ticks every 2 ms, so
  `wait_readable(TICK)` could become "wait on the reader's channel with
  timeout".
- Backpressure: bounded queues reproduce today's "don't read the socket while
  stdout is full" rule (**inference**).
- Cancellation: pipe readers end when ConPTY closes or the child exits
  [17][18]. The console stdin reader is the hard one: it is blocked in a
  synchronous console read. Either use `CancelSynchronousIo` (driver support
  not guaranteed [42]), or have the reader wait on the console input handle
  with a timeout and read only when `GetNumberOfConsoleInputEvents` shows
  records (**inference**).
- Pros: matches Microsoft's ConPTY guidance [17], and works with
  portable-pty's anonymous pipes unchanged.
- Cons: more threads, and ordering and shutdown need explicit testing. Unix
  code is unaffected.

**B. `WSAEventSelect` + `WaitForMultipleObjects` (+ helper threads for
pipes).**

- One wait over socket events, the console input handle, the child process
  handle, and an event set by pipe-reader threads [20][39].
- Pros: one wait point, which resembles the bridge's `poll` shape.
- Cons:
  - `FD_WRITE` is edge-like [39], unlike `POLLOUT`'s level semantics. The
    bridge's write-readiness logic must change on Windows.
  - Console output still needs a writer thread.
  - Signalled console input can carry non-text records [40].
  - The ConPTY pipe still needs a thread, because pipes are not waitable
    [18][20].

**C. IOCP / overlapped I/O.**

- Sockets support overlapped I/O, and polling uses AFD on an IOCP [32].
- Anonymous pipes do not support overlapped I/O [18], and portable-pty
  creates exactly those [34]. Overlapped I/O on our side of ConPTY would need
  climon2 to create the pipes itself: a named pipe with `FILE_FLAG_OVERLAPPED`
  on our end and a synchronous handle given to ConPTY (**inference**; [16]
  restricts only the handles passed to ConPTY). That means replacing
  portable-pty's pipe setup.
- Console handles remain synchronous.
- Pros: one completion port, and the most scalable option.
- Cons: the most Windows-only code, and it diverges furthest from the Unix
  readiness model, so a bridge rewrite is needed.

**D. `polling` crate as the Windows wait primitive (variant of A/B).**

- `Poller` registers AF_UNIX sockets (AFD), the console input handle and the
  child process handle (`add_waitable`). Reader and writer threads `post()`
  completion packets to wake the loop [32][33].
- Readiness semantics are oneshot/level per `PollMode`, closer to `poll` than
  `WSAEventSelect` is (**inference** from [32]).
- Already in the lockfile, but only as a dev-dependency. Adopting it adds a
  production dependency that uses undocumented AFD calls [32]. On Unix it
  would not be compiled if it is a `cfg(windows)` dependency.

**Not viable as the sole mechanism:**

- mio and tokio: no Windows AF_UNIX [30][31].
- filedescriptor's `poll`: `WSAPoll`, so sockets only [34].
- `PeekNamedPipe` busy-polling at 2 ms is possible for the daemon's ConPTY
  output and needs no reader thread. But it can block in a multi-threaded
  process [41], and climon2's daemon already has threads.

### 3.4 Keeping Unix paths unchanged

The seams already isolated in `climon2-platform` can take Windows bodies with
the same signatures:

- `connection::{connect, wait}`: socket2 + `WSAPoll`.
- `runtime::{bind, validate_stale_socket, RuntimeDirectory, private_directory}`:
  socket2 bind, reparse-tag check, SID/DACL.
- `peer::{accept, authenticate}`: no-op, or PID→SID.

Some shared code is not seam-isolated today; it uses `libc` and `std::os::unix`
directly:

- `climon2-client/src/transport.rs` (`libc::pollfd`, `UnixStream`)
- `climon2-client/src/bridge.rs` (`libc::pollfd`, `fcntl`, `O_NONBLOCK`)
- `climon2-daemon/src/pty.rs` (`libc::poll`, `fcntl`, `UnixStream`,
  `CommandExt`)
- `climon2-cli/src/list.rs` (`libc::ENOENT | libc::ECONNREFUSED`)

Each needs either a new platform seam or a Windows-only sibling file. Which of
those is a policy decision for later tickets. The `list.rs` errno match is the
only one with a shared, behaviour-identical alternative (`ErrorKind`, §1.4).

## Windows 11 vs. GitHub-hosted runners

| Topic | Windows 11 (target) | GitHub `windows-2022` / `windows-2025` (CI) |
| --- | --- | --- |
| OS build | 22000 (21H2) … 26100 (24H2), 26200 (25H2), 26300 (26H2), 28000 (26H1) [23] | 10.0.20348 (Server 2022) [22b]; 10.0.26100 (Server 2025) [22]. Server 2025 shares 24H2's build number. Server 2022 predates every Windows 11 build. |
| x64 Windows 11 image | n/a | None. Hosted Windows labels are `windows-2025`, `windows-2025-vs2026`, `windows-2022` (x64) and `windows-11-arm` (arm64 only) [21]. |
| User privilege | Normal user, UAC on | "configured to run as administrators with User Account Control (UAC) disabled" [21]. Any test that another user is denied, or that a non-admin can't bypass the DACL, needs a second, non-admin account created in the job (**inference**). Admin-held `SeRestorePrivilege` makes DACL tests pass trivially [37]. |
| Filesystem | NTFS system drive | `%TEMP%` lives on the system drive (**inference**). The 2025 image notes "D: drive will use ReFS instead of NTFS on Windows images from November 30, 2026" [22]. AF_UNIX socket files are NTFS reparse points [1]. Whether ReFS supports them is unverified, so tests that bind sockets under the workspace (`D:\a\…`) may differ from `%TEMP%`. **Needs CI evidence.** |
| AF_UNIX / ConPTY availability | Present | AF_UNIX is present on both server builds, since the std tests gate on build 17134 and later [27] (**inference** for exact behaviour parity). ConPTY's documented server minimum is Server 2019 [16]. |
| Missing-path connect error, stale-file semantics, delete-while-listening | Not documented | Not documented. Measure on both runner images, and treat them as proxies for Windows 11, not proof. |

## Open questions for CI evidence

1. The `connect` error code for a non-existent path, compared with a stale
   socket file, on Server 2022 and Server 2025 (§1.4).
2. Whether `DeleteFile` succeeds on a socket file whose listener is still
   open, and whether a later `connect` then fails.
3. Whether the socket file inherits a protected owner-only DACL from its
   directory, and whether a non-admin second user is denied `connect`.
4. Whether `SIO_AF_UNIX_GETPEERPID` returns the connecting PID on both runner
   builds.
5. AF_UNIX `bind` on the ReFS `D:` drive after 2026-11-30.
6. Whether `CancelSynchronousIo` unblocks a thread in a synchronous console
   `ReadFile` under a ConPTY harness.

## Sources

1. Microsoft Command Line blog, "AF_UNIX comes to Windows" (2017-12-19). https://devblogs.microsoft.com/commandline/af_unix-comes-to-windows/
2. Ibid. (`sun_path` is "a Win32 UTF-8 file system path").
3. `afunix.h` (mingw-w64 mirror of the Windows SDK header): `UNIX_PATH_MAX 108`, `SOCKADDR_UN`, `SIO_AF_UNIX_*`. https://mingw.googlesource.com/mingw-w64/+/refs/heads/v14.x/mingw-w64-headers/include/afunix.h
4. socket2 0.6.5 source: `src/sys/windows.rs` (`unix_sockaddr`, `poll_connect` with `WSAPoll`, `OwnedSocket` conversions), `src/socket.rs` (`connect_timeout`), `src/lib.rs` (`Domain::UNIX`), `Cargo.toml` (windows-sys range). `~/.cargo/registry/src/*/socket2-0.6.5/`
5. Rust 1.96.0 std source: `library/std/src/os/windows/net/{addr,stream,listener}.rs`, `#![unstable(feature = "windows_unix_domain_sockets", issue = "150487")]`.
6. [MS-FSCC] 2.1.2.1 Reparse Tags: `IO_REPARSE_TAG_AF_UNIX` 0x80000023. https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fscc/c8e77b37-3909-4fe6-a4ea-2b9d423b1ee4
7. windows-sys 0.61.2: `Win32/System/SystemServices/mod.rs` `IO_REPARSE_TAG_AF_UNIX`.
8. Rust 1.96.0 std source: `library/std/src/sys/fs/windows.rs` (`metadata`, `lstat`, `File::file_attr`, `FileType::new`); `library/std/src/os/windows/fs.rs` (`MetadataExt::file_attributes`).
9. docker/secrets-engine PR #677 (Windows 11 manual run; stale file → `WSAECONNREFUSED` 10061; missing → `fs.ErrNotExist`). https://github.com/docker/secrets-engine/pull/677. **Third-party.**
10. Rust 1.96.0 std source: `library/std/src/sys/io/error/windows.rs` `decode_error_kind` (`ERROR_FILE_NOT_FOUND`/`ERROR_PATH_NOT_FOUND`→`NotFound`, `WSAECONNREFUSED`→`ConnectionRefused`, `WSAEWOULDBLOCK`→`WouldBlock`).
11. padcom/pico-lang issue #109 (reports 10061 for a missing socket path). https://github.com/padcom/pico-lang/issues/109. **Third-party, unconfirmed.**
12. Rust 1.96.0 std source: `library/std/src/sys/io/error/unix.rs` (`ENOENT`→`NotFound`, `ECONNREFUSED`→`ConnectionRefused`).
13. windows-sys 0.61.2: `Win32/Networking/WinSock/mod.rs` (`AF_UNIX = 1`, `SIO_AF_UNIX_GETPEERPID`, `SIO_AF_UNIX_SETBINDPARENTPATH`, `SIO_AF_UNIX_SETCONNPARENTPATH`, `SOCKADDR_UN`).
14. Microsoft Learn, Privilege Constants (`SE_RESTORE_NAME`). https://learn.microsoft.com/en-us/windows/win32/secauthz/privilege-constants
15. Microsoft Learn, WSAPoll function. https://learn.microsoft.com/en-us/windows/win32/api/winsock2/nf-winsock2-wsapoll
16. Microsoft Learn, CreatePseudoConsole function. https://learn.microsoft.com/en-us/windows/console/createpseudoconsole
17. Microsoft Learn, Creating a Pseudoconsole session. https://learn.microsoft.com/en-us/windows/console/creating-a-pseudoconsole-session
18. Microsoft Learn, Anonymous Pipe Operations. https://learn.microsoft.com/en-us/windows/win32/ipc/anonymous-pipe-operations
19. portable-pty 0.9.0 source: `src/win/conpty.rs` (`Pipe::new()` for stdin/stdout), `src/win/mod.rs` (`kill` → `TerminateProcess`).
20. Microsoft Learn, WaitForMultipleObjects function. https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-waitformultipleobjects
21. GitHub Docs, GitHub-hosted runners reference (labels; "run as administrators with User Account Control (UAC) disabled"). https://docs.github.com/en/actions/reference/runners/github-hosted-runners
22. actions/runner-images, Windows Server 2025 readme ("OS Version: 10.0.26100"; ReFS `D:` notice). https://github.com/actions/runner-images/blob/main/images/windows/Windows2025-Readme.md
    22b. actions/runner-images, Windows Server 2022 readme ("OS Version: 10.0.20348"). https://github.com/actions/runner-images/blob/main/images/windows/Windows2022-Readme.md
23. Microsoft Learn, Windows 11 release information (builds 22000, 22621, 22631, 26100, 26200, 26300, 28000). https://learn.microsoft.com/en-us/windows/release-health/windows11-release-information
24. uds_windows 1.2.1 source: `src/stdnet/net.rs` (`UnixStream::pair` via a temp-dir listener). https://crates.io/crates/uds_windows
25. Cygwin patches, "Cygwin: AF_UNIX: use FILE_OPEN_REPARSE_POINT when needed". https://sourceware.org/pipermail/cygwin-patches/2020q4/010670.html. **Third-party.**
26. Wine merge request on AF_UNIX reparse points (wine-gitlab list). https://list.winehq.org/hyperkitty/list/wine-gitlab@list.winehq.org/message/PGSJHPJZUMO7PVRJMZZ7BW5FLLD433T2/. **Third-party.**
27. Rust 1.96.0 std tests: `library/std/tests/windows_unix_socket.rs` (remove-before-bind; double bind fails; long path fails; gated on Windows 10 1803 / build 17134).
28. Microsoft Learn, Windows Sockets Error Codes. https://learn.microsoft.com/en-us/windows/win32/winsock/windows-sockets-error-codes-2
29. crates.io API, `uds_windows` (max_version 1.2.1, updated 2026-03-14; reverse deps include zbus, async-io, smol). https://crates.io/crates/uds_windows
30. mio 1.2.4 source: `src/net/mod.rs` (`#[cfg(unix)] mod uds`).
31. tokio 1.53.1 source: `src/net/mod.rs`, `src/macros/cfg.rs` (`cfg_net_unix!` = `#[cfg(all(unix, feature = "net"))]`).
32. polling 3.11.0 source: `src/iocp/mod.rs` (AFD strategy and its stability note), `src/os/iocp.rs` (`PollerIocpExt::{post, add_waitable}`; waitables include console inputs, events, processes).
33. async-io 2.6.0: `examples/windows-uds.rs` (registers `uds_windows` sockets with `Async`/polling). https://crates.io/crates/async-io
34. filedescriptor 0.8.3 source: `src/windows.rs` (`Pipe::new` → `CreatePipe`; `poll_impl` → `WSAPoll`; `socketpair_impl` over `AF_INET` loopback).
35. Microsoft Learn, DACL for a New Object. https://learn.microsoft.com/en-us/windows/win32/secauthz/dacl-for-a-new-object
36. Microsoft Learn, Access Rights and Access Masks (security-descriptor functions per object type). https://learn.microsoft.com/en-us/windows/win32/secauthz/access-rights-and-access-masks
37. Microsoft Learn, Privilege Constants (`SE_BACKUP_NAME`, `SE_RESTORE_NAME`, `SE_TAKE_OWNERSHIP_NAME`, `SE_DEBUG_NAME`). https://learn.microsoft.com/en-us/windows/win32/secauthz/privilege-constants
38. WineHQ bug 60201, "Implement SIO_AF_UNIX_GETPEERPID ioctl". https://list.winehq.org/hyperkitty/list/wine-bugs@list.winehq.org/thread/D5XYEW4MKQX2RCOLQVOMH6I2HBCXGDGY/. **Third-party.**
39. Microsoft Learn, WSAEventSelect function. https://learn.microsoft.com/en-us/windows/win32/api/winsock2/nf-winsock2-wsaeventselect
40. Microsoft Learn, Reading Input Buffer Events. https://learn.microsoft.com/en-us/windows/console/reading-input-buffer-events
41. Microsoft Learn, PeekNamedPipe function. https://learn.microsoft.com/en-us/windows/win32/api/namedpipeapi/nf-namedpipeapi-peeknamedpipe
42. Microsoft Learn, Canceling Pending I/O Operations. https://learn.microsoft.com/en-us/windows/win32/fileio/canceling-pending-i-o-operations
