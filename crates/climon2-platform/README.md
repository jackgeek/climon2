# climon2-platform

Shared macOS, Linux and Windows seams (ADR 0003). `home`, `private`,
`logging`, `runtime`, `connection`, `waiting`, `process` (identity and `exit`),
`presentation`, `peer`, `daemon`, `pty`, `terminal`, `library`, `notifier`
and `native_path` build on Windows, and `handoff` and `command` exist only
there (see [Windows](#windows)). Neutral contracts remain in `climon2-core`. No runtime
dispatch or shell-specific logic.

## Runtime and peers

- `runtime::canonical_home(&Path) -> io::Result<PathBuf>` creates and
  canonicalizes the resolved absolute home **once per invocation**.
- `home` is where every executable finds its homes (#155), so the CLI, the
  client executable and the monitor share one rule for each:
  `home::named()` is the climon2 home the environment names (`CLIMON2_HOME`,
  else `.climon2` in `HOME`; absolute; nothing touched), `home::canonical()`
  that home created and canonicalized through `canonical_home`,
  `home::default_executable_home()` the executable home an installer uses
  without `--dir` (`.local/share/climon2` in an absolute `HOME`; nothing
  touched), and
  `home::executable_layout(VersionBinary)` the install layout around the
  running executable's real path. Its outer error is the system failing to
  name the executable; its inner `ContractError`, a path outside any version
  folder, so a caller can word that refusal itself. On Windows the
  registered native file `src/home/windows.rs` (#313) names the variables:
  the climon2 home is `.climon2` in `USERPROFILE`, still overridden by
  `CLIMON2_HOME`, and the default executable home is `climon2` in
  `LOCALAPPDATA`.
- `RuntimeDirectory::prepare(&canonical_home)` creates/checks the core-derived
  `/tmp/climon2-<euid>-<hash8>` directory. `path()` and
  `socket_path(&SessionId)` expose its paths; the socket limit is 103 bytes
  (107 on Windows, where the directory is `%LOCALAPPDATA%r<hash8>`).
  The directory is never deleted.
- `private` (ADR 0003, "Private files and folders") holds every create of a
  file climon2 makes and every check of a folder it manages; `runtime`,
  logging, the PTY allocation lock, session metadata and the client
  library's log all go through it.
- `private::private_directory(&Path)` creates mode 0700, then checks with
  `symlink_metadata`: real directory, effective UID owner, no group/other bits.
  Existing unsafe directories are refused, never repaired.
- `private::create_new_file(&Path) -> io::Result<File>` is the one create for
  every file climon2 makes (#312): `create_new`, mode 0600 under the umask,
  and `O_NOFOLLOW`, so never over an existing file or through a symlink. Logs
  and the client library's log use it directly.
- `private::create_private_file(&Path) -> io::Result<File>` creates through
  `create_new_file`, then sets mode exactly 0600 whatever the umask. A file it
  cannot make private is removed again. The daemon publishes session
  metadata through it (#311).
- On Windows the registered native file `src/private/windows.rs` (#339) gives
  the same three functions their bodies. Every file and folder gets the
  private descriptor when it is created: owner the process token's user SID
  (set explicitly, since an administrator's full token otherwise makes
  `BUILTIN\Administrators` the owner; see ADR 0003, "Why the owner is
  explicit") and a protected DACL with one ACE, that SID with full
  control, inheritable on folders. `create_new_file` calls `CreateFileW` with
  `CREATE_NEW | FILE_FLAG_OPEN_REPARSE_POINT`, so any existing name,
  including a dangling link, is refused and nothing is created at a link's
  target; a name holding a NUL is refused. `create_private_file` is the same
  function, as Windows has no umask. `private_directory` creates the folder
  when missing, then opens the name itself and requires a directory that is
  not a reparse point of any tag and whose owner and DACL read back exactly as
  the private descriptor; otherwise it fails with the "unsafe runtime
  directory" error and changes nothing.
- `runtime::PATH_MAX` is the longest path the system accepts, in bytes with
  its NUL.
- `runtime::bind(&Path)` binds under umask 077. Bind before starting worker
  threads: umask is process-wide; the lock serializes only this helper's calls.
- `runtime::{SocketListener,SocketStream}` name the Unix transport types
  without exposing libc types in callers' signatures.
- `connection::nobody_listening(&io::Error)` is true when a connect found the
  socket missing (ENOENT) or refusing (ECONNREFUSED), read as std's
  `NotFound` and `ConnectionRefused` kinds; on Windows a missing path and a
  stale socket both refuse with `WSAECONNREFUSED`. Only these can begin to
  prove a session stale (#311).
- `connection::send(&SocketStream, &[u8])` sends what a socket takes now and
  never raises `SIGPIPE` in the client library's embedder: Linux sends with
  `MSG_NOSIGNAL`, and macOS needs `connection::quiet(&SocketStream)` once per
  socket to set `SO_NOSIGPIPE` (a no-op on Linux and Windows, which has no
  `SIGPIPE`). Native files `src/connection/{linux,macos,windows}.rs` (#312,
  #318).
- `peer::accept(&SocketListener)` returns only authenticated streams.
  `peer::{uid,authenticate}` support already accepted streams; authentication
  must happen before hello and callers must drop rejected streams.
- Linux authenticates with `SO_PEERCRED`; macOS uses `getpeereid`. Both require
  the effective user's UID.

## Processes and startup

- `process::start_time(pid) -> io::Result<Option<u64>>` uses `proc_pidinfo` on
  macOS and procfs boot time/start ticks on Linux; timestamps are Unix-epoch
  microseconds. Permission/inspection errors are **not** death.
  Linux treats both a missing stat path and `ESRCH` during its read as absence:
  reaping can invalidate an already open procfs descriptor.
- `process::is_same_daemon(metadata)` uses epoch start time on macOS and
  `daemon_linux_identity` (boot ID plus start ticks) on Linux. Linux's epoch
  conversion is clock-sensitive and is not stale-cleanup authority.
  Missing stable identity makes Linux cleanup fail safely, retaining metadata.
- `process::{wait,try_wait}` own reaping by PID and return core `ExitStatus`
  without portable-pty's lossy signal mapping. Do not also wait through
  portable-pty or `std::process::Child`.
- `process::signal_group(group, Signal)` supports Hangup, Interrupt, Terminate,
  Kill, rejects 0, 1 (launchd) and out-of-range IDs, so `kill(0)`/`kill(-1)`
  are unreachable, and tolerates a nonexistent group. A unit test checks the
  refusal without sending a signal: under a mutant it would reach `kill(-1)`.
- `process::Terminal::from_path(path)` identifies a terminal (a PTY slave) by
  its character device number; anything else is refused.
- `process::TerminalSession::new(leader, terminal)` is the session a command
  leads on its PTY. Candidates for a session kill come **only** from kernel
  lists: `proc_listpids` on macOS, or procfs filtered by terminal and recorded
  job group on Linux; every candidate must also satisfy
  `getsid == leader` and not be the caller, and is signalled through a
  non-zero-PID-only kill. macOS asks the kernel for scoped candidate lists;
  Linux must enumerate procfs, but filters both candidates and session
  membership before signaling. Zombies are not live Linux candidates.
  macOS detaches the terminal from the session when the leader exits, so
  `observe()` records the members' job groups while the leader lives.
  `kill()` observes once more, then SIGKILLs members in bounded rounds until
  none remain; it works after the leader was reaped. Processes that started
  their own session (setsid/daemonize) left the terminal and are never
  candidates. Callers reap only their own children.
- `process::hangup(&mut session, close_master)` observes job groups, closes
  every PTY-master handle through the caller's closure, waits the real five
  seconds, reaps an exited leader, kills the session, then returns exact status.
- `process::validate_cwd(&Path)` must run immediately before portable-pty spawn,
  which otherwise silently falls back to HOME. As with ordinary `chdir`, a
  concurrent directory removal can still race validation.
- `process::before_fork()` must run on the forking thread immediately before
  every fork (pre_exec spawns, portable-pty spawn). macOS runs libnotify's fork
  handler in the child, which traps (SIGILL) or hangs if another thread was
  initialising libnotify at the fork; this call waits that initialisation out.
  Linux needs no libnotify preparation. `daemon::spawn` calls it itself.
- `process::descriptor_limit() -> u64` is the soft `RLIMIT_NOFILE`, read
  afresh; one that cannot be read is 0. `process::out_of_descriptors(&io::Error)`
  is true for EMFILE and ENFILE. `process::exit_immediately(code) -> !` ends
  the process with `_exit`, running no exit handlers and flushing nothing
  (#312).
- `daemon::spawn(Command, configure)` calls `configure(&mut Command, fd: u32)`
  so the caller can append its readiness descriptor argument. It uses exec via
  `Command::pre_exec(setsid)`, inherited pipe, null stdin/stdout, and the caller's
  stderr configuration. No PTY-command environment is changed.
- `PendingDaemon::{id,wait_ready}` exposes the PID and enforces an absolute real
  five-second deadline. `wait_ready(&[metadata, socket])` returns
  `Readiness::{Ready,Failed { code, message }}`; failure/EOF/timeout kills and
  reaps the daemon, then removes the supplied paths in order. Dropping a pending
  startup also kills/reaps it; callers that want file cleanup must call
  `wait_ready`. Successful startup leaves the daemon running.
- Daemons exclusively adopt the inherited fd through
  `unsafe ReadinessWriter::from_inherited(fd)`, which marks it close-on-exec
  before PTY spawn. `send(&Readiness)` uses a bounded internal binary protocol:
  Ready byte, or Failed byte + big-endian code/UTF-8 length + message (4096-byte
  maximum, codes 125/126/127). Failure messages must contain no command content.

## Terminal

- `terminal::require_ttys()` checks stdin/stdout; `input()` duplicates stdin
  into an owned `File`; `size(&File)` uses TIOCGWINSZ and core's 80x24 fallback.
- `RawModeGuard::new(&File)` enters raw mode only after Attached.
  Drop and a chained panic hook restore termios only. One raw-mode
  owner is permitted in a process. No screen reset sequences are emitted.
- `Signals::client()` defers HUP/INT/TERM and coalesces WINCH.
  `termination_signal()` and `take_resize()` are polled by the event loop,
  which must reread the size and suppress unchanged resizes.
  `Signals::daemon()` ignores HUP/INT/PIPE and defers TERM. Registrations are
  unregistered on drop; no terminal operations run in signal handlers.
- `Nonblocking::new(input, output)` makes both borrowed terminal handles
  nonblocking for the client bridge. It saves both flags before changing
  either, because stdin and stdout may share one open-file description, and
  restores them on drop through owned duplicates.
- `terminal::hung_up(&io::Error)` is true for a read that failed because the
  terminal was lost (EIO). Callers treat it like end of file.
- `terminal::Bridged::new(input)` is the terminal a client bridges (#321):
  stdin and an owned duplicate of stdout under `Nonblocking`, read and written
  through `Read` and `Write`, with `watch_input()` and `watch_output()` as
  their wait slots and `size()`. A read that fails with `hung_up` reads 0
  bytes, the hang-up.
- `pty::allocation()` serializes PTY pair allocation per user (Darwin's
  EREDRIVEOPEN). `pty::set_nonblocking(BorrowedFd)` makes a PTY master
  nonblocking and keeps its other status flags; `pty::output_ended(&io::Error)`
  is true for a master read that failed because the command's side is gone
  (EIO), the end of its output (#311).

## Waiting for readiness

- `waiting::wait(&mut [Watch], Duration) -> io::Result<usize>` is the one wait
  every tick loop and deadline uses (#310). It waits until a watched handle is
  ready or the timeout passes, and returns how many are ready. The timeout is
  rounded up to whole milliseconds, so a deadline's last fraction is waited
  rather than spun, and saturates at `i32::MAX` milliseconds. Every failure,
  EINTR included, is returned; each caller decides whether a signal ends its
  wait.
- `Watch::new(BorrowedFd, Interest)` watches one handle for `Read`, `Write` or
  `ReadWrite`. `Watch::idle()` keeps a slot's place and is never ready. After
  a wait, `ready()` reports anything (what was wanted, a hang-up or an error)
  and `readable()` reports readable data only.
- `connection::wait` and the daemon readiness pipe wait through it;
  `connection::Interest` is `waiting::Interest`.

## Client libraries

- `notifier::Notifier` is the client library's pollable descriptor, raised and
  cleared by `raise`, `clear` and `set(bool)`: its notification descriptor
  (`new`) and each instance's waker (`waker`, never blocking either way).
  Linux uses one nonblocking `eventfd`, macOS a close-on-exec pipe
  (`DESCRIPTORS` counts 1 and 2). `descriptor()` is the raw descriptor the C
  ABI hands out; `as_fd()` is the one a wait watches. Native files
  `src/notifier/{linux,macos}.rs` (#312). `notifier::Waker` is the waker's
  type (one more `Notifier` on Unix) and `Waker::watch()` its slot in a wait;
  `notifier::wait_raised(i32, Duration)` waits on a notification descriptor
  the library handed out.
- `native_path::NativePath::new(&Path).native()` is a path as it crosses the
  native interface (`cm1_path`), and `native_path::parse(encoding, bytes)`
  reads one back: native Unix bytes as `CM1_NATIVE_UNIX`, refusing NUL,
  relative paths and `PATH_MAX` bytes or more; on Windows UTF-16LE code units
  as `CM1_NATIVE_WINDOWS` (`src/native_path/windows.rs`, #318). Each refuses
  the other encoding, and `native_path::RULE` words the refusal.
- `library::Loader::new(ExecutableHome)` loads the client libraries of one
  climon2 executable home. `climon2_core::layout::ExecutableHome` is absolute
  and canonical by construction and owns each library's path,
  `<home>/<version>/<library file>`. `load(&VersionName)` opens that path with
  `dlopen(RTLD_NOW | RTLD_LOCAL)`, never `current`, another version or a
  search path, and never closes a handle, including one it then refuses. The
  first successful load of a version is shared: later loads return the same
  `&'static Library`; two first loads racing on different threads may both
  open and validate it.
- A refusal is not remembered, so a later load tries again. A library refused
  before the dynamic loader opened it (missing library or dependency,
  incompatible architecture) can load once the installation is repaired, and
  a contained bootstrap failure can pass. Once a file is open, the process
  keeps its image, so a later load repeats any refusal of its symbol or
  description.
- A refusal is a `LoadError` naming the requested file, with one `Failure`
  per explicit cause. After a failed `dlopen` the loader's own checks decide,
  never `dlerror()` text, which is only quoted: a file it cannot open,
  missing or unreadable, is a `MissingLibrary`; a file whose header is not an
  x86-64 ELF header on Linux (`library::is_x86_64_elf`, which xtask's release
  checks share), or that has no slice dyld would load on macOS
  (`macho_best_slice_in_fd`), is an `IncompatibleArchitecture`; otherwise a
  `MissingDependency`. Then, with the handle kept: on macOS, an image other
  than the requested file, found through `DYLD_LIBRARY_PATH`, is a
  `SubstitutedImage`, checked before anything else: `dladdr` names the image
  of the bootstrap, and without one the requested file must be among dyld's
  images. No `climon2_client_get_interface` is a `MissingSymbol`; a refused
  revision 1 is an `UnsupportedRevision`; a contained bootstrap failure
  (`CM1_INTERNAL_ERROR`) that cleared the output, which the loader sets to a
  value no bootstrap returns, is a recoverable `BootstrapFailure`, and one
  that did not is a `MalformedDescription`; a release text
  other than the version is a `WrongRelease`; and any other answer without a
  description, another revision or a nonzero reserved field, a release text
  that is null or longer than 255 bytes (no version folder name is longer), a
  missing requirement or an incomplete function table is a
  `MalformedDescription`.
- Requirements must be present: each one an instance needs is nonzero (its
  local allocation, owner stack and instance descriptors, and a worker stack
  whenever it needs workers), and the reserved field is zero. The loader sets
  no upper bound: admission (#120) refuses requirements incompatible with the
  admitted limits.
- `Library` exists only validated: `requirements()`, `functions()` (a
  `Functions` table whose eleven entries cannot be absent) and `bootstrap()`
  for observers of the C ABI itself. The monitor (#106), the client
  executable (#100, #101), the Library-layer host, xtask's release checks and
  the PERF-01 harness (`climon2-measure`, to read requirements) load through
  it. Environment variables such as `LD_PRELOAD`,
  `LD_AUDIT`, `LD_LIBRARY_PATH` and `GLIBC_TUNABLES` are neither cleared nor
  a reason to refuse.

## Windows

Registered native files give the Windows bodies (#316; ADR 0003, "Windows
I/O and transport"; docs/for-humans/windows/lifecycle.md, "Identity and liveness"):

- `src/runtime/windows.rs`: `RuntimeDirectory::prepare` uses
  `%LOCALAPPDATA%\c2` as its root (created when missing and not checked;
  `LOCALAPPDATA` must be absolute) and names the directory `<hash8>`, with no
  uid. The directory is prepared by `private::private_directory`, so an
  unsafe one fails with the "unsafe runtime directory" error; the socket file
  inherits its one ACE. The socket-path budget is 107 bytes; over it,
  `socket_path` and `bind` fail, and `exec`'s own failure is 125.
  `SocketStream` is `socket2::Socket`, and `bind` returns a `SocketListener`
  whose `accept` returns the stream and its address, and whose
  `set_nonblocking` is std's. `SocketFreshness` does nothing: nothing ages
  `%LOCALAPPDATA%`. `validate_stale_socket` accepts a
  missing path and a reparse point tagged `IO_REPARSE_TAG_AF_UNIX` (std
  reports a socket as a regular file), and refuses anything else, links and
  junctions included.
- `src/waiting/windows.rs`: `Watch` and `wait`. `Watch::new(BorrowedSocket,
  Interest)` and, on every platform, `Watch::stream(&SocketStream, Interest)`
  watch sockets with `WSAPoll`; `Watch::event(BorrowedHandle)` watches a
  manual-reset event with `WaitForMultipleObjects` (#321); one wait watches
  sockets or events, never both. `Watch::idle()` keeps a slot that is never
  ready, as on Unix. A Windows wait can return up to a clock tick early with
  nothing ready, so `wait` waits again until the timeout has passed. One
  `WaitForMultipleObjects` takes at most 64 events, so a wait on more watches
  the first 64, and any event already signalled, beyond them too, ends it at
  once (#327, the monitor's instances).
- `src/process/windows.rs`: `start_time`, `is_same_process` and
  `is_same_daemon`. A daemon is its PID plus its `GetProcessTimes` creation
  time in epoch microseconds. Only "no such process"
  (`ERROR_INVALID_PARAMETER`) or a process that has exited while its object
  is still held reads as gone; any other failure, such as access denied, is
  an error and never evidence of death. `descriptor_limit` is the kernel's
  per-process handle table, 2^24 entries, as Windows has no soft limit.
  `exit(status)` ends the process at once with all 32 bits of `status`,
  through `TerminateProcess` (#323), as the client exits; on Unix it exits
  with the status's low eight bits. `exit_immediately` also ends it with
  `TerminateProcess` and all 32 bits (#327); on Unix it is `_exit`, which
  skips exit handlers.
- `src/presentation/windows.rs`: `local_datetime` uses the system's offset
  for the instant; `columns` is the visible console window's width, or 80
  when stdout is not a console.

For the session daemon and headless `exec` (#317; docs/for-humans/windows/lifecycle.md,
"Starting the daemon" and "Starting the command"):

- `src/daemon/windows.rs`: `spawn` starts the daemon with one
  `CreateProcessW`: detached, breaking away only when the caller is in a
  job, suspended until `IsProcessInJob` shows it left every job, with null
  standard handles, its own folder as working directory, exactly the
  variables set on the `Command`, and a one-entry handle list holding the
  readiness pipe, whose handle value it gets as `--readiness-fd`. A spawn
  refused in a job, or a daemon still in one, fails ("cannot leave the job")
  and the daemon is ended. The readiness reply is the same bytes as on Unix,
  read on a thread of its own until `STARTUP_TIMEOUT`.
  `ReadinessWriter::from_inherited` takes the handle value and clears its
  inheritance.
- `src/pty/windows.rs`: `ConPty::load` loads `conpty.dll` by full path from
  a folder, its imports limited to that folder and System32, and refuses a
  folder without `OpenConsole.exe`. `open` creates the session job
  (`BREAKAWAY_OK` and `KILL_ON_JOB_CLOSE`) and the pseudoconsole;
  `Opened::start` sets the given variables in this process, clears an
  inherited "ignore Ctrl+C", starts a `command::Launch` with one
  `CreateProcessW` carrying the pseudoconsole and the job, inheritance off
  and null standard handles, and releases the pseudoconsole. `Hosted` reads output and writes
  input through one thread each and a bounded channel; `output_ended` is
  `ERROR_BROKEN_PIPE`. `try_wait` reads the leader's handle; `close`, `end`
  (the job terminated with `0xC000013A`, then the leader's status) and
  `finish` are hang-up's steps. The output thread drops the pseudoconsole's
  start-up negotiation from the start of the output (#325), and the input
  thread first answers its device attributes request, so the answer to the
  command's own request reaches the command (#328).
- `src/command/windows.rs` (#322; docs/for-humans/windows/lifecycle.md,
  "Command resolution and arguments"), a module only Windows has:
  `search(command, cwd, path, pathext)` finds the file a command names, in
  each absolute `PATH` folder in order and in each the `PATHEXT` extensions
  in order (`.COM;.EXE;.BAT;.CMD` when unset or empty), never the current
  directory or a relative entry; a name with an extension is tried as typed
  first; a command holding `\`, `/` or `:` is found from `cwd` instead.
  `launch(command, arguments, cwd)` searches with this process's `PATH` and
  `PATHEXT` and returns the `Launch` `CreateProcessW` gets: the resolved path
  as the application and the command as typed, then the arguments, quoted by
  the C runtime's rules; or, for a `.bat` or `.cmd`, the system folder's
  `cmd.exe` given `/e:ON /v:OFF /d /c ""<script>" <arguments>"` with Rust std
  1.96.0's batch escaping. It refuses with `Refusal::NotFound`, or
  `Refusal::Unrepresentable` for a batch argument holding CR, LF or NUL.
  `tests/command_windows.rs` checks the search order and refusals, and runs a
  corpus of `cmd.exe`'s special characters through a real `.cmd` in the
  pinned ConPTY: `cmd.exe` gets the command line std gives it, and the
  arguments reach the program the script runs unchanged.
- `src/peer/windows.rs`: `accept` and `authenticate` check nothing: Windows
  AF_UNIX has no peer credentials, and the private runtime directory keeps
  other users out.
- `src/terminal/windows.rs`: a detached daemon's `Signals`, which never
  report a termination signal.
- `src/logging/windows.rs`: `capture_stderr` makes a pipe the standard error
  handle and copies it into the log until dropped.

For the client library and `c2 kill` (#318; library embedding, "Windows"):

- `src/library/windows.rs`: the loader opens the library with
  `LoadLibraryExW` and `LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR |
  LOAD_LIBRARY_SEARCH_SYSTEM32`, never frees it, and refuses it as a
  `SubstitutedImage` when `GetModuleFileNameW` names another file. A failed
  load is a `MissingLibrary` when the file cannot be opened, an
  `IncompatibleArchitecture` for `ERROR_BAD_EXE_FORMAT` or a PE machine other
  than x86-64, and otherwise a `MissingDependency`; no bootstrap export is a
  `MissingSymbol`.
- `src/notifier/windows.rs`: the notification descriptor is an unnamed,
  non-inheritable manual-reset event whose handle travels in the `int32_t`
  output (ADR 0008); the waker is a loopback UDP socket connected to itself,
  the only kind of handle that can wake the instance's `WSAPoll`.
- `src/connection/windows.rs`: sends with no setup.
- `src/native_path/windows.rs`: `CM1_NATIVE_WINDOWS` paths, at most 32767
  code units, with unpaired surrogates kept.
- `src/process/windows.rs` adds `out_of_descriptors` (`WSAEMFILE` or
  `ERROR_TOO_MANY_OPEN_FILES`).

`tests/pty_windows.rs` proves the PTY seam against the pinned ConPTY that
`climon2-test-term` fetches and verifies; headless `exec`'s tests in
`climon2-cli` prove the daemon's start. `tests/notifier_windows.rs`,
`tests/native_path_windows.rs` and the new cases in `tests/connection_windows.rs`
and `tests/process_windows.rs` prove the client library's seams;
`climon2-client`'s `tests/library_windows.rs` proves the loader against the
built library and altered copies of it.

For interactive `exec` and `attach` (#321; docs/for-humans/windows/lifecycle.md,
"Hand-offs" and "Client console"):

- `src/terminal/windows.rs`: `require_ttys` needs console handles for stdin
  and stdout; `input` duplicates the console input; `size` is the console
  window's. `RawModeGuard` clears processed, line and echo input and quick
  edit, sets VT input, window input and the extended flags, adds VT
  processing and no automatic newline to output, and sets both code pages to
  UTF-8; its drop, or the panic hook, restores all four. `Bridged` reads
  console input on one thread and writes output on another, each through a
  bounded channel and each signalling an event that `watch_input` and
  `watch_output` return. Only key-down characters become input, as UTF-8; a
  window-size record sets the resize `Signals::take_resize` reports; output
  is cut only between whole characters (`climon2_core::utf8`) and drained
  when the bridge ends. The client's `Signals` take Ctrl+C as signal 2 and
  close, logoff and shutdown as signal 1, whose handler holds the process
  until the client has exited 129; Ctrl+Break does nothing yet (#326).
- `src/handoff/windows.rs`: `replace` starts a child, waits for it and exits
  with its raw status, as the launcher hands off to the CLI;
  `wait_through_console_events` is its handler routine alone: Ctrl+C and
  Ctrl+Break are the child's, and a close waits three seconds, then exits
  129.
- `src/daemon/windows.rs` adds `stop`, which ends a ready daemon and, with it,
  its session job.
- `src/notifier/windows.rs` adds `Notifier::watch` and `watch_raised`, the
  wait slots of a notifier's event and of a notification descriptor.

`tests/terminal_windows.rs` and `tests/handoff_windows.rs` host this crate's
test binary in the pinned ConPTY under the harness's Windows Terminal model:
the modes, code pages and their restoration after a drop and after a panic;
key-down-only UTF-8 input; resize; whole characters and drained output; the
console preflight; Ctrl+C, Ctrl+Break and close; and the hand-off's statuses,
console events and close wait.

## Behavioral evidence

`tests/runtime.rs`: `issue18_private_directory_refuses_symlink_file_and_permissive_mode_without_repair`,
`issue18_home_is_created_then_canonicalized_and_socket_is_private_and_authenticated`,
`issue311_a_private_file_is_new_owner_only_and_never_a_symlink`,
`issue312_a_new_file_keeps_the_umask_and_never_replaces_a_file`.

`tests/connection.rs`: `issue19_socket_connection_waits_for_real_readiness_until_the_deadline`,
`issue311_only_a_missing_or_refusing_socket_says_nobody_listens`. The unit tests
in `src/pty.rs` cover `set_nonblocking` and `output_ended` (#311).

`tests/home.rs` (#155) runs each case in a child of the test binary, with
the environment or path under test:
`issue155_the_environment_names_the_climon2_home`,
`issue155_the_canonical_climon2_home_is_created_and_resolved`,
`issue155_an_executable_finds_its_layout_from_its_own_path` (a hard link of
the test binary in a version folder), and
`issue155_an_executable_outside_a_version_folder_has_no_layout`, and (#313)
`issue313_the_environment_names_the_default_executable_home`.
`tests/home_windows.rs` (#313, Windows only, run by CI's Windows Test job)
proves the Windows names the same way:
`issue313_the_profile_names_the_climon2_home_on_windows` and
`issue313_local_application_data_names_the_default_executable_home_on_windows`.

`tests/private_windows.rs` (#339, Windows only, run by CI's Windows Test job
and the Windows gate) reads descriptors back independently, with Windows
PowerShell's `Get-Acl`, and needs the right to create symbolic links (CI and
an elevated session have it):
`a_dangling_file_link_at_the_final_name_is_refused_and_nothing_appears_at_its_target`,
`an_existing_file_is_refused_and_kept`,
`a_name_holding_a_nul_is_refused_and_nothing_is_created_at_its_prefix`,
`a_created_file_carries_exactly_the_private_descriptor`,
`a_prepared_folder_carries_exactly_the_private_descriptor_and_preparing_again_keeps_it`,
`a_folder_whose_parent_is_missing_is_not_prepared`,
`a_folder_with_the_inherited_descriptor_is_refused_and_left_unrepaired`,
`a_folder_with_an_extra_ace_is_refused_and_left_unrepaired`,
`a_file_standing_in_for_a_folder_is_refused`,
`a_junction_or_directory_link_standing_in_for_a_folder_is_refused`,
`a_general_log_is_a_private_file_in_private_folders` and
`a_junction_or_directory_link_standing_in_for_logs_is_refused_and_pruning_leaves_its_target_alone`.

The #316 tests are Windows only and run by CI's Windows Test job and the
Windows gate. Tests that need their own `LOCALAPPDATA` run their body again
in a child of the test binary.
`tests/runtime_windows.rs`:
`the_runtime_directory_is_the_home_hash_in_c2_under_local_app_data_and_private`,
`an_unsafe_runtime_directory_is_refused_and_left_unrepaired`,
`a_relative_local_app_data_is_refused`,
`a_socket_path_over_107_bytes_is_refused_and_fails_with_125`,
`bind_admits_a_107_byte_path_and_refuses_108_bytes`,
`a_session_socket_binds_and_connects_in_the_real_runtime_directory`,
`a_stale_socket_or_a_missing_path_may_be_removed_as_a_socket` and
`anything_but_a_socket_file_is_refused_as_a_stale_socket`.
`tests/connection_windows.rs`:
`a_bound_socket_accepts_a_connection_that_carries_bytes_both_ways`,
`a_connection_waits_for_real_readiness_until_the_deadline`,
`one_wait_reports_each_ready_socket_until_the_timeout`,
`a_hung_up_peer_is_ready`,
`a_listener_is_readable_once_a_connection_is_pending`,
`a_missing_or_stale_socket_is_refused_and_says_nobody_listens` (a missing
directory instead fails with `WSAENETDOWN`, which proves nothing),
`a_live_socket_file_can_be_deleted_and_then_refuses_new_connections` and
`binding_over_a_stale_socket_file_fails_until_it_is_deleted`
(`WSAEADDRINUSE`).
`tests/process_windows.rs`:
`a_live_process_has_its_creation_time_as_epoch_microseconds`,
`a_daemon_is_the_same_only_with_its_pid_and_its_creation_time`,
`a_gone_process_is_not_the_same_daemon`,
`an_exited_process_whose_handle_is_held_is_not_the_same_daemon` and
`a_process_the_caller_may_not_open_is_never_taken_for_a_gone_one` (an empty
process DACL, opened without `SeDebugPrivilege`).
`tests/presentation_windows.rs`:
`a_creation_time_is_shown_in_the_systems_local_time` (checked against .NET)
and `redirected_output_is_80_columns_wide`.
`tests/cross_account_windows.rs`:
`a_second_account_is_denied_the_runtime_directory_logs_and_metadata` logs
on as a second, non-administrator account and finds it denied the runtime
directory, creating a file in it, a session log, a metadata file and the
`logs` folder, while it can read an ordinary file beside them. CI's Windows
jobs create that account; elsewhere the test is skipped with that reason,
and on CI it fails without the account.

`tests/process.rs`: `issue26_identity_uses_start_time_and_refuses_zero_or_pid_reuse`,
`issue19_waitpid_preserves_exit_and_real_signal_status`,
`issue19_hangup_escalates_group_after_real_five_seconds`,
`issue19_closing_real_controlling_pty_delivers_hangup`,
`issue19_signal_group_signals_the_group_and_tolerates_a_nonexistent_one`,
`issue19_kill_session_kills_every_job_but_not_new_sessions`,
`issue19_kill_session_spares_the_calling_process`,
`issue19_kill_session_only_signals_session_members_on_the_given_terminal`,
`issue19_kill_session_never_enumerates_beyond_terminal_and_observed_groups`,
`issue19_terminal_must_be_a_character_device`,
`issue15_bad_cwd_is_rejected_before_pty_can_fall_back`,
`issue312_the_descriptor_limit_is_the_soft_limit`,
`issue312_only_emfile_and_enfile_are_out_of_descriptors`,
`issue312_an_immediate_exit_ends_with_its_code_and_flushes_nothing`.

On Linux x86_64,
`procfs_reaping::process_reaped_between_procfs_open_and_read_is_absent` traces
an owned helper to reap its target between the real procfs open and read.
It exercises the public identity API without replacing procfs or mocking I/O.

`tests/daemon.rs`: `issue19_an_inherited_readiness_pipe_closes_on_exec`,
`issue19_a_readiness_descriptor_that_is_not_open_is_refused`,
`issue19_descriptor_3_carries_readiness`,
`issue19_dropping_a_pending_daemon_kills_and_reaps_it`,
`issue19_the_longest_readiness_failure_message_arrives_whole`,
`issue19_daemon_exec_is_detached_and_readiness_preserves_launch_failure`,
`issue19_empty_readiness_kills_daemon_and_cleans_metadata_before_socket`,
`issue19_readiness_uses_real_five_second_absolute_deadline`.

`tests/library.rs` (Fault layer, #84) loads the
[client library stand-in](../../GLOSSARY.md#testing), compiled by each test
through `climon2_fixtures::stand_in` into a private executable home, exact or
with one defect, through `Loader`. Its calls abort, so a test that survives a
refusal proves nothing called them.

| Scenario | Test |
|---|---|
| LIB-02 missing library, no substituted release: nothing loaded, not even the version `current` names | `missing_library_is_refused_without_substituting_another_release` |
| LIB-01 same-release instances share one load: two loads share one library; its requirements and table are its own | `two_loads_of_one_version_share_one_library` |
| LIB-02 missing symbol, handle kept | `library_without_the_bootstrap_is_refused_and_stays_loaded` |
| LIB-02 unsupported revision; a bootstrap answer without a description is malformed | `bootstrap_without_a_revision_one_description_is_refused` |
| LIB-02 contained bootstrap failure: refused, and a later load succeeds once it passes | `contained_bootstrap_failure_is_refused_and_recoverable` |
| LIB-02 contained bootstrap failure that leaves its output: a malformed description, handle stays loaded | `contained_bootstrap_failure_that_leaves_its_output_is_a_malformed_description` |
| LIB-02 wrong release at the release text's bounds (255 bytes wrong, 256 bytes or none malformed) | `library_describing_another_release_is_refused` |
| LIB-02 malformed description: revision, reserved, each missing requirement (a worker, even one, without a stack), incomplete table | `malformed_description_is_refused_before_any_instance` |
| LIB-02 requirements present at their limits: each nonzero minimum accepted at 1; no workers need no worker stack | `requirements_are_accepted_at_their_minimums` |
| LIB-02 missing dependency and incompatible architecture, decided by the loader's own checks; nothing loaded | `library_the_dynamic_loader_cannot_load_is_classified_by_the_loaders_checks` |
| LIB-01 macOS: a `DYLD_LIBRARY_PATH` decoy without the bootstrap is a substituted image, not a missing symbol | `library_substituted_through_dyld_library_path_is_refused_even_without_a_bootstrap` |

`tests/terminal.rs`: `issue20_real_pty_raw_mode_restores_on_drop_and_panic_without_screen_sequences`,
`issue20_real_pty_size_and_zero_fallback`,
`issue20_signals_coalesce_resize_and_report_exit_signal`.

`tests/waiting.rs` (#310): `issue310_one_wait_reports_each_ready_handle_until_the_timeout`,
`issue310_a_hung_up_peer_is_ready`, `issue310_idle_slots_wait_out_the_timeout`.

`tests/notifier.rs` (#312): `issue312_a_notifier_is_readable_from_a_raise_until_cleared`,
`issue312_a_waker_never_blocks`, `issue312_a_send_to_a_closed_peer_fails_with_epipe`.

Tests use isolated private directories. Direct socket fixtures use short `/tmp`
paths so mutation workers and long checkouts do not exceed the socket limit.
Ignored libtest functions are owned subprocess fixtures, executed by the
behavioral tests, not skipped requirements. Production `/tmp` naming and home
isolation are independently covered by core's layout tests.
