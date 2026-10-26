# Research: PTY and terminal-testing capabilities of `portable-pty` on macOS

Ticket: #15. Question: what do `portable-pty` and the surrounding Rust ecosystem
provide for climon2's first release (macOS + bash), and what constraints do they
impose? This note reports facts and tradeoffs. It does not make architecture
decisions.

**Method.**
- **Crate source:** the published crates.io tarballs and the upstream repos.
- **macOS man pages:** read on the host, macOS 26.7.1 (`man 2 fork`, `man 4 unix`, and so on).
- **Windows:** the Microsoft Learn docs and the Microsoft devblog cited in §5.
- **Experiments:** small programs on macOS 26.7.1 with rustc 1.96.0,
  `portable-pty` 0.9.0, `nix` 0.31.3, `rustix` 1.1.5 and `vt100` 0.18.1.
  Experiment results are marked **[observed]**.
- **Versions and dates:** from the crates.io API on 2026-10-26.

---

## 1. `portable-pty`

### Version and maintenance

| Fact | Source |
|---|---|
| The latest release is **0.9.0**, published 2025-02-11. Before that came 0.8.1 (2023-03-13) and 0.8.0 (2023-02-17). | crates.io API `/api/v1/crates/portable-pty/versions` |
| It lives in the wezterm monorepo (`wezterm/wezterm`, directory `pty/`). It is MIT licensed and written by Wez Furlong. | `pty/Cargo.toml` |
| The `pty/` directory still gets commits on `main`, but none of them have been released. Examples: "pty: windows: fix kill()" (#7709, 2026-06-07), "pty: Replace winapi with windows-sys" (#8073, 2026-08-25) and "Fix typo psuedo => pseudo" (#7178, 2026-09-12). `main` still says `version = "0.9.0"`. | GitHub commits API with `path=pty`; `pty/Cargo.toml` on `main` |
| The last tagged wezterm release is `20240203-110809-5046fc22`. | GitHub releases API |
| 0.9.0 depends on `nix ^0.28` (features `term` and `fs`), `filedescriptor ^0.8.3`, `anyhow`, `downcast-rs`, `libc` and `serial2`. On Windows it also uses `winapi`, `winreg` and `shared_library`. | crates.io dependencies API for 0.9.0 |

**Implications for climon2**
- Fixes do get merged, but releases are rare. Some fixes on `main` are missing
  from 0.9.0. For example, on Windows `kill()` returned `Err` on success
  (#7709). If climon2 needs such a fix before the next release, it needs a git
  dependency or `[patch.crates-io]`.
- `MasterPty::get_termios()` returns a `nix::sys::termios::Termios` from nix
  0.28 (`src/lib.rs`). That makes nix 0.28 part of portable-pty's public API.
  If climon2 uses a newer nix (0.31 is current), the two `Termios` types are
  different and can't be used interchangeably.

### Spawning: `CommandBuilder` (unix, `src/cmdbuilder.rs` and `src/unix.rs`)

- **argv is passed through exactly.** `CommandBuilder::new(prog)` plus
  `.arg/.args`, or `from_argv(Vec<OsString>)`, keeps the given string as
  `arg0`. It resolves the program through `PATH` and calls
  `std::process::Command::new(resolved).arg0(args[0]).args(&args[1..])`.
  Nothing goes through a shell.
  - **[observed]** `printf "[%s]" "a b" "" '$HOME' '*'` printed
    `[a b][][$HOME][*]`.
- **PATH lookup is done by portable-pty, not by `execvp`.**
  - It reads the **builder's** `PATH` value.
  - Relative `PATH` entries are joined to the child's cwd.
  - `./x` and `../x` are resolved only against the child's cwd.
  - If lookup fails, it returns an `anyhow` error **before fork**, with a
    message such as "not executable", "does not exist" or "is a directory".
  - If the exec itself fails, `cmd.spawn()` returns an `io::Error`.
- **cwd gotchas**
  - If you don't call `.cwd()`, the child runs in **`$HOME`**, not in the
    caller's cwd. `as_command` falls back to `get_home_dir()`.
    **[observed]** `pwd` printed `/Users/jackallan`.
  - If the cwd you pass is not a directory, portable-pty **silently replaces it
    with `$HOME`** (`.filter(|dir| Path::new(dir).is_dir())`).
    **[observed]** `cwd("/does/not/exist")` printed `/Users/jackallan` and
    exited 0.
  - **Implication:** climon2 must pass `std::env::current_dir()` explicitly. If
    it wants an error instead of a silent fallback, it has to check the cwd
    itself first.
- **The environment is almost exact, with one exception: `SHELL`.**
  - The base is `std::env::vars_os()` (`get_base_env`). `.env(k, v)`,
    `.env_remove(k)` and `.env_clear()` work on top of it.
  - `SHELL` is always set. `get_base_env` adds it from the passwd database if
    it is missing. `as_command` also runs `cmd.env("SHELL", shell)` after
    `env_clear()`.
  - **[observed]** Both `env_clear()` and `env_remove("SHELL")` still left
    `SHELL=/bin/zsh` in the child.
  - **Implication:** with "full env passthrough", the only difference on macOS
    is that `SHELL` gets added when the parent didn't have it. Setting
    `CLIMON2_SESSION_ID`, `CLIMON2_SESSION_DEPTH` and a default `TERM` with
    `.env()` works.
- **Child setup before exec.** The `pre_exec` closure in `src/unix.rs` does the
  following, in order:
  1. Resets SIGCHLD, SIGHUP, SIGINT, SIGQUIT, SIGTERM and SIGALRM to `SIG_DFL`.
  2. Clears the signal mask.
  3. Calls `setsid()`.
  4. Calls `ioctl(0, TIOCSCTTY, 0)`, unless `set_controlling_tty(false)` was
     set.
  5. Calls `close_random_fds()`.
  6. Applies the optional `umask`.

  **[observed]** `ps` inside the child showed PGID = PID = TPGID and a TTY of
  `ttys0NN`. In other words, the child is a session leader and the foreground
  process group of the PTY.
- **Fork-safety caveat.** `close_random_fds()` calls `std::fs::read_dir("/dev/fd")`
  inside `pre_exec`, which allocates.
  - The std docs for `CommandExt::pre_exec` say the closure must be
    *async-signal-safe* and specifically warn against allocating
    (`library/std/src/os/unix/process.rs`). The macOS `fork(2)` CAVEATS section
    says the same.
  - `main` still has this code.
  - Because a `pre_exec` closure is present, std can't use `posix_spawn` and
    does a real fork+exec instead.
  - **Implication:** suppose the daemon calls `spawn_command` while another
    thread holds an allocator lock. The forked child could then deadlock. This
    is unlikely but possible. One way to avoid it is to spawn the PTY child
    while the daemon is still single-threaded.
- **File descriptors.** The master and slave fds are opened `FD_CLOEXEC`, and
  the child closes every fd above 2. The daemon's socket fds therefore don't
  leak into the user's command.

### Resize, read, write

- **Resize.** `MasterPty::resize(PtySize { rows, cols, pixel_width, pixel_height })`
  calls `ioctl(TIOCSWINSZ)` on the master. The kernel then sends SIGWINCH to
  the foreground process group. `get_size()` calls `TIOCGWINSZ`.
  - **[observed]** After `resize(40x100)`, a `trap WINCH` in the child fired,
    and `stty size` printed `40 100`.
- **Initial size.** `openpty` takes an initial size, so the child can start at
  the client's real size.
- **Reading.** `try_clone_reader()` returns a blocking `Box<dyn Read + Send>`
  that is a dup of the master fd.
  - **`EIO` is turned into `Ok(0)`, i.e. EOF.**
  - There is no async API. On unix, `as_raw_fd()` exposes the master fd, so a
    caller can set `O_NONBLOCK` and poll it.
- **Writing.** `take_writer()` can be called **only once**.
  - **When the writer is dropped, it writes `\n` followed by the VEOF
    character** (`UnixMasterWriter::drop`).
  - **[observed]** Dropping the writer after sending `hello` to `cat` produced
    `hello\r\n^D\b\bhello\r\n`.
  - **Implication:** the daemon must keep the writer for the whole session. If
    it drops the writer when a client disconnects, the child receives a newline
    and EOF.
- **End of output on macOS. [observed]**
  - After the child exited, the reader saw EOF even though the parent still
    held the slave fd.
  - The reader received all 300 000 bytes the child wrote just before `exit 3`,
    in 3 of 3 runs.
  - These are observations on macOS 26.7.1, not documented guarantees. Linux
    may behave differently, so Ubuntu needs its own test.
- **Discovery helpers.** `process_group_leader()` (via `tcgetpgrp(master)`) and
  `tty_name()` are unix-only. They may be useful for discovery and diagnostics.

### Exit status and signals (needed for 128+signal)

`portable_pty::ExitStatus` stores `code: u32` and `signal: Option<String>`.

When `From<std::process::ExitStatus>` sees a signal, it does two things
(`src/lib.rs`):
- It stores **the `strsignal(sig)` text**, not the signal number.
- It sets `code` to `status.code().unwrap_or(1)`, which is `1`.

So **the signal number is lost**. **[observed]**

| child | `exit_code()` | `signal()` | std `ExitStatusExt::signal()` |
|---|---|---|---|
| `exit 7` | 7 | None | None |
| `kill -TERM $$` | 1 | `"Terminated: 15"` | `Some(15)` |
| `kill -KILL $$` | 1 | `"Killed: 9"` | `Some(9)` |
| `kill -INT $$` | 1 | `"Interrupt: 2"` | `Some(2)` |

**Workarounds**
1. **Downcast to the std child.** On unix, `SlavePty::spawn_command` returns a
   boxed `std::process::Child`, and `Child: Downcast`. Reborrow it as
   `&mut dyn Child`, then call
   `.downcast_mut::<std::process::Child>()`. From there, std `wait()` and
   `ExitStatusExt::signal()` give the number.
   - `downcast_mut` is defined on `dyn Child`, not on
     `dyn Child + Send + Sync`, hence the reborrow.
   - **[observed]** This works.
2. **Wait on the pid yourself.** Call `waitpid` on `process_id()`, for example
   with `nix::sys::wait`.
3. **Parse the trailing `: N`** from the `strsignal` text. This is fragile,
   because the text format is platform-specific.

**Killing the child.** On unix, `ChildKiller::kill()` sends **SIGHUP**. It then
polls `try_wait` up to five times, sleeping 50 ms between polls (about 200 ms
in total). If the child is still alive, it sends SIGKILL. It never sends
SIGTERM. A killer from `clone_killer()` sends SIGHUP only.

---

## 2. Daemonizing the process that owns the PTY (macOS)

**OS facts**

Sources: the macOS man pages for `fork(2)`, `daemon(3)`, `setsid(2)`, `tty(4)`
and `termios(4)`, plus the std `CommandExt` and `sys/process/unix` source.

- **`fork(2)` CAVEATS (macOS):** "To be totally safe you should restrict
  yourself to only executing async-signal safe operations until such time as
  one of the exec functions is called. ... If you need to use these frameworks
  in the child process, you must exec. In this situation it is reasonable to
  exec yourself."
- **`daemon(3)` (macOS):**
  - It uses fork, so the same caveats apply.
  - The page says: "On Mac OS X, the use of this API is discouraged in favor of
    using launchd(8)".
  - By default it changes directory to `/` and redirects stdio to `/dev/null`.
- **`setsid(2)`:**
  - The caller becomes the session leader and process-group leader "and has no
    controlling terminal".
  - It fails with **EPERM** if the caller is already a process-group leader.
    That is why the classic recipe forks first.
- **`tty(4)`:** "The current system does not allocate a controlling terminal to
  a process on an open() call: there is a specific ioctl called TIOSCTTY."
  - So a daemon that has called `setsid` does not accidentally get a controlling
    terminal when it opens a PTY.
  - The classic second fork exists to prevent exactly that. For that purpose
    it isn't needed on macOS.
- **`termios(4)` "Modem Disconnect":** on hangup, SIGHUP goes to the
  controlling process.
  - When the user's terminal closes, its session gets SIGHUP. A daemon in its
    own session does not.
  - Inside the PTY, the child is the session leader. It gets SIGHUP when the
    daemon closes the master.

**What Rust std provides**
- `CommandExt::pre_exec` (stable since 1.34) is `unsafe`, and the closure must
  be async-signal-safe. `setsid()` is on the POSIX async-signal-safe list.
- `CommandExt::process_group(0)` (stable since 1.64) gives a new process group,
  but **not** a new session.
- `CommandExt::setsid(bool)` is **unstable** (`process_setsid`, #105376).
- std's unix `posix_spawn` path:
  - std skips `posix_spawn` when there is any `pre_exec` closure.
  - It uses `POSIX_SPAWN_SETSID` only on Linux/glibc. Elsewhere, including
    macOS, a `setsid` request falls back to fork+exec.
  - It uses `POSIX_SPAWN_SETPGROUP` for `process_group`.

**Crates**

| crate | version | last update | approach |
|---|---|---|---|
| `daemonize` | 0.5.0 | 2023-02-25 | the classic in-process fork, `setsid`, fork sequence |
| `fork` | 0.11.0 | 2026-10-23 | thin wrappers around `fork`, `setsid` and `daemon` |

Both fork without exec. That is the pattern the macOS man pages warn about for
processes with threads or frameworks loaded.

**Implications for climon2 (tradeoffs, not decisions)**
- **Spawn self.** Something like
  `Command::new(current_exe()).arg(<hidden daemon subcommand>)`, with
  `pre_exec(|| { libc::setsid(); Ok(()) })`, redirected stdio, and an explicit
  cwd if needed.
  - This follows Apple's "exec yourself" advice.
  - It avoids fork-safety problems in a multithreaded or tokio CLI.
  - It can be tested like any other binary.
  - The daemon starts fresh and single-threaded, so it can spawn the PTY child
    before it starts other threads (see §1).
  - Costs: one extra exec. The CLI must also send the command spec (argv, cwd,
    env, size) to the daemon, through argv, env, a pipe or the socket.
- **Fork in-process** (`daemonize`, `fork`, `nix::unistd::fork`).
  - This is only safe if the CLI is single-threaded at fork time and the child
    does only async-signal-safe work.
  - That is hard to guarantee once logging, a tokio runtime, or lookups such as
    `std::env::current_exe` are involved.
- **Either way, close or redirect the daemon's stdio.** A daemon that keeps the
  user's tty fds open keeps a reference to the user's terminal, and its output
  can leak into that terminal after the client has exited.
- **`launchd`** is Apple's recommended mechanism for long-running daemons. It
  doesn't fit per-command session processes well, but it is worth knowing
  about.

---

## 3. Client-side raw mode, save/restore, and SIGWINCH

| crate | version (updated) | raw mode | size | SIGWINCH |
|---|---|---|---|---|
| `crossterm` | 0.29.0 (2025-04-05), MSRV 1.63 | `enable_raw_mode` / `disable_raw_mode` | `terminal::size()` | through its event reader (`Event::Resize`) |
| `nix` | 0.31.3 (2026-05-11) | `termios::{tcgetattr, cfmakeraw, tcsetattr}` | `ioctl` macros | `nix::sys::signal` (raw `sigaction`) |
| `rustix` | 1.1.5 (2026-09-16) | `termios::{tcgetattr, Termios::make_raw, tcsetattr}` | `termios::tcgetwinsize` | none |
| `signal-hook` | 0.4.5 (2026-10-04), MSRV 1.66 | n/a | n/a | `iterator::Signals`, `flag`, `low_level::pipe`; `signal-hook-mio` 0.3.0 adds mio support |

**crossterm** (`src/terminal/sys/unix.rs` and `file_descriptor.rs`):
- It saves the termios from before raw mode in a **process-global**
  `Mutex<Option<Termios>>`. `disable_raw_mode` restores exactly that value,
  with `TCSANOW`.
- It uses stdin if stdin is a tty, and otherwise opens `/dev/tty`.
- It uses rustix by default, with libc as an option.
- It does **not** install a panic hook or signal handlers to restore the
  terminal. The application must restore the terminal itself:
  - on panic
  - on SIGTERM and SIGHUP
  - on SIGTSTP and SIGCONT, if it supports job control
- Its SIGWINCH support is part of its event source (`signal-hook` + mio, in
  `src/event/source/unix/mio.rs`). Using that means reading stdin **through
  crossterm's parser**, which turns the bytes into key and mouse events.
- **Implication:** climon2 forwards raw bytes, and crossterm's event reader
  would consume and reinterpret them. A byte-transparent proxy has two options:
  - use crossterm only for raw mode and size
  - skip crossterm and use rustix or nix termios behind its own RAII guard

  Either way, SIGWINCH comes from `signal-hook`, followed by a fresh
  `TIOCGWINSZ` call.

**Other notes**
- **[observed]** `rustix` 1.1.5 with only the `net` feature **does not
  compile**, because `crate::timespec` is gated behind other features. Adding
  `process` fixed it. This only matters if climon2 uses rustix directly with a
  minimal feature set.
- SIGWINCH signals coalesce. The robust pattern is: signal → query the size
  again → send the latest size. Don't send one message per signal.

---

## 4. Driving a PTY from automated tests

### Expect-style crates

| crate | version (updated) | PTY backend | platforms | notes |
|---|---|---|---|---|
| `expectrl` | 0.9.0 (2026-05-11) | its own `ptyprocess` 0.5 (nix 0.26) on unix; `conpty` 0.5 on Windows | unix and Windows | Generic `Session<P, S>::new(process, stream)`. `expect()` needs `S: Read + Write + NonBlocking`. Has regex matching and `interact`; async is optional. |
| `rexpect` | 0.7.1 (2026-05-14), edition 2024, MSRV 1.85 | its own PTY on nix 0.31 | unix only (`categories = os::unix-apis`) | `spawn_bash` writes a temporary rcfile that **sources `/etc/bash.bashrc` and `~/.bashrc`**, sets `TERM=""`, and resets PS1. `StreamSession::new(reader, writer, opts)` accepts any `Read` and `Write`. |

**Implications**
- **Neither crate drives `portable-pty` out of the box.**
  - expectrl needs an adapter that implements `NonBlocking`. On unix that can
    be `O_NONBLOCK` on `as_raw_fd()`. Elsewhere it can be a reader thread plus
    a channel.
  - rexpect's `StreamSession` accepts portable-pty's reader and writer as they
    are, but rexpect is unix-only.
- **rexpect's `spawn_bash` is not a controlled shell startup**, because it
  loads the user's dotfiles. For a controlled startup, climon2 would run
  `bash --noprofile --norc`, or `--rcfile <fixture>`, with `HOME` and `ENV`
  under its control.
- **macOS `/bin/bash` is GNU bash 3.2.57. [observed]**
  - It supports `--norc`, `--noprofile` and `--rcfile`.
  - It lacks newer bash features such as `PROMPT_COMMAND` arrays and `PS0`.
  - Homebrew bash, if installed, may come first on `PATH`.
- **An in-house matcher** on top of `portable-pty` is a reasonable alternative:
  a read loop, an accumulating buffer, a regex and a deadline. It keeps one PTY
  implementation for both the product and the tests.

### Screen-emulation crates

| crate | version (updated) | API | answers terminal queries? | notes |
|---|---|---|---|---|
| `vt100` | 0.18.1 (2026-10-19), edition 2024, MSRV 1.85 | `Parser::new(rows: NonZeroU16, cols: NonZeroU16, scrollback)`, `process(&[u8])`, `screen().contents()`, `cursor_position()`, `contents_formatted()`, `contents_diff()`, `alternate_screen()` | **No.** It does not handle DSR/CPR (`CSI 6 n`); those go to `Callbacks::unhandled_csi`, and the caller must reply. | Small; depends on `vte` 0.15. Since 0.18, sizes are `NonZeroU16`. **[observed]** `"hello\x1b[2;3Hworld"` gave contents `"hello\n  world…"` and cursor `(1,10)`. |
| `alacritty_terminal` | 0.26.0 (2026-04-06), MSRV 1.85 | a full `Term<T: EventListener>`, with a grid and scrollback | **Yes.** For DA, DSR/CPR and similar queries it emits `Event::PtyWrite(String)` (`term/mod.rs`). | Heavier: pulls in polling, rustix-openpty, signal-hook and regex-automata. It ships its own `tty` module (PTY and ConPTY), which a fixture would not use. |
| `termwiz` | 0.23.3 (2025-03-20) | `Surface` is a change-based model. `escape::parser::Parser` produces `Action`s. | No terminal-state machine is included. The emulator that applies Actions (`wezterm-term`) is **not published on crates.io**. | 36 dependency entries. Same author and repo as portable-pty. |

**Implications for a generic terminal fixture**
- **`vt100`** is the lightest option and gives deterministic screen snapshots
  for assertions. The fixture must answer queries itself, at least CPR, through
  the callbacks.
- **`alacritty_terminal`** answers queries out of the box. That matters for
  programs that wait for a reply, and on Windows (see §7). The price is more
  weight and more API churn.
- Both consume the raw bytes from the PTY reader. So either one can sit behind
  a single fixture API, for example: `send_keys`,
  `wait_for_text(regex, deadline)`, `snapshot()`, `resize()` and
  `exit_status()`.

---

## 5. Unix-domain socket peer credentials

**macOS**
- **`getpeereid(3)`** (macOS man page): "returns the effective user and group
  IDs of the peer connected to a UNIX-domain socket". The socket must be a
  connected or listening `SOCK_STREAM` socket. The server sees the client's
  credentials as they were at `connect(2)`.
- **`unix(4)`** (macOS): the `LOCAL_PEERCRED` socket option returns a
  `struct xucred`, containing the effective uid and the group list.
- **std:** `UnixStream::peer_cred()` is **nightly-only**: feature
  `peer_credentials_unix_socket`, tracking issue #42839. According to its
  rustdoc it is available on Apple, Linux and the BSDs
  (`library/std/src/os/unix/net/ucred.rs`).
- **nix 0.31:**
  - `nix::unistd::getpeereid(fd)` (`#[cfg(bsd)]`, which includes Apple).
    **[observed]** It returned `(Uid(501), Gid(20))`.
  - `sockopt::LocalPeerCred` (`apple_targets`). **[observed]** It returned uid
    501.
  - `sockopt::LocalPeerPid` (`apple_targets`). **[observed]** It returned the
    peer's pid.
- **rustix 1.1.5:** `net::sockopt::socket_peercred` is `#[cfg(linux_kernel)]`
  only. I found no `getpeereid` or `LOCAL_PEERCRED` wrapper for Apple.
- **Implication:**
  - On stable Rust for macOS, a peer UID check today means nix, or calling
    `libc::getpeereid` directly.
  - On Ubuntu, `SO_PEERCRED` is available through both nix and rustix.
  - Mode 0700 on the runtime directory is a second, independent safeguard.

**Windows (future)**
- **AF_UNIX on Windows.** Source: Microsoft devblogs, "AF_UNIX comes to
  Windows", 2017-12-19.
  - It arrived in Insider build 17063.
  - Only `SOCK_STREAM` is supported.
  - There is **no ancillary data**: no `SCM_RIGHTS` and no `SCM_CREDENTIALS`.
  - There is no `socketpair` and no autobind.
  - The socket file is an NTFS reparse point.
  - Microsoft says security comes from file and directory permissions on the
    socket path.
- **Peer process on Windows AF_UNIX: the `SIO_AF_UNIX_GETPEERPID` ioctl**
  (via `WSAIoctl`).
  - It returns the peer's **PID**, not its SID.
  - It is in the Win32 metadata: `windows-sys` exposes
    `Win32::Networking::WinSock::SIO_AF_UNIX_GETPEERPID`.
  - I could not find it on the Microsoft Learn "Winsock IOCTLs" page, so
    consider it thinly documented.
  - Getting a SID means `OpenProcess` → `OpenProcessToken` →
    `GetTokenInformation(TokenUser)`. A PID can be reused, so this can race.
- **Rust std has no AF_UNIX support on Windows.** Third-party crates such as
  `uds_windows` (1.2.1, 2026-03-14) fill the gap.
- **Named pipes** (Microsoft Learn API pages):
  - `GetNamedPipeClientProcessId` returns the client's PID.
  - `ImpersonateNamedPipeClient` lets the server impersonate the client and
    read its token. That gives the SID directly, without the PID race.
  - A security descriptor passed to `CreateNamedPipe` can restrict the pipe to
    the owner's SID.

---

## 6. macOS `sun_path` limit and runtime-dir location

**Facts**
- **The limit is 103 bytes.** `unix(4)` on macOS says addresses are
  "filesystem pathnames of at most 104 characters", in `char sun_path[104]`.
  With the NUL terminator, the usable length is **103 bytes**.
  - **[observed]** std `UnixListener::bind` accepted a 103-byte path and
    rejected 104 and 105 bytes with "path must be shorter than SUN_LEN".
- **`$TMPDIR` is per-user and long.** It lives under `/var/folders/…/T/`, the
  same path `getconf DARWIN_USER_TEMP_DIR` reports.
  - On this host it is **49 bytes**, trailing slash included:
    `/var/folders/s_/rtjmw7ks1tqb2fsrcqzntd2c0000gn/T/`.
  - Adding `climon2-501/` (12 bytes) leaves about 42 bytes for the socket file
    name.
  - A UUID (36) plus `.sock` (5) is 41 bytes. That fits, but with almost no
    slack. Longer names, or a nested directory, would not fit.
- **`XDG_RUNTIME_DIR` is normally unset on macOS. [observed]** The XDG Base
  Directory spec requires that directory to be owned by the user, mode 0700,
  and tied to the login session.
- **`/tmp` is a symlink to `private/tmp`. [observed]** `ls -ld /tmp` shows
  `/tmp -> private/tmp`.
  - `realpath` and `canonicalize` add 8 bytes (`/private`).
  - Canonicalized paths won't string-compare equal to `/tmp/...`.
- **`/tmp` is shared.** It is world-writable with the sticky bit. Another user
  can create `/tmp/climon2-<uid>/` first, or make it a symlink.
  - Safe creation means: create with mode 0700, then use `lstat` to check that
    it is a directory and not a symlink, that it is owned by this uid, and that
    its mode is 0700.
  - If any check fails, refuse to use the directory.

**Implications (tradeoffs, not decisions)**
- `/tmp/climon2-<uid>/` gives short, predictable paths, but needs the ownership
  and symlink checks above.
- `$TMPDIR` is already per-user and private, but its long prefix constrains
  naming.
- Short socket names (an ID rather than a description) keep both options under
  103 bytes.
- `chdir` into the directory and binding a relative path avoids the limit, but
  changes process-global state.

---

## 7. Constraints for future ConPTY support via `portable-pty`

Sources: `src/win/*.rs` in portable-pty 0.9.0, and the Microsoft Learn ConPTY
docs.

- **Minimum OS.** Windows 10 1809 or Windows Server 2019 (`CreatePseudoConsole`
  docs).
  - If `kernel32` lacks the API, portable-pty panics through `.expect()`.
  - It **prefers a side-loaded `conpty.dll`** found through the normal DLL
    search. That raises DLL search order and supply-chain questions.
- **Flags.** portable-pty creates the console with
  `PSEUDOCONSOLE_INHERIT_CURSOR | PSEUDOCONSOLE_RESIZE_QUIRK | PSEUDOCONSOLE_WIN32_INPUT_MODE`.
  - The Microsoft docs say that with INHERIT_CURSOR, the caller "should be
    prepared to respond to the request for the cursor state in an asynchronous
    fashion ... Failure to do so may cause the calling application to hang."
  - **Implication:** on Windows, the daemon or test fixture **must answer CPR
    (`ESC[6n`)**. `alacritty_terminal` does this. With `vt100` it has to be
    done by hand.
  - The `WIN32_INPUT_MODE` flag means win32-input-mode key encoding may be in
    play. Verify this when Windows work starts.
- **No signals.**
  - On Windows, `ExitStatus` is always `with_exit_code(GetExitCodeProcess)`.
    There is no signal, so 128+signal has no direct equivalent.
  - `kill()` is `TerminateProcess(handle, 1)`. In 0.9.0 its result check is
    inverted; this is fixed on `main` (#7709).
  - Ctrl+C reaches the child as input byte `0x03` sent through the
    pseudoconsole. It does not arrive as a POSIX signal.
- **Resize.** Done with `ResizePseudoConsole`.
  - Pixel sizes are accepted but not passed on.
  - `get_size()` returns a cached value, not one queried from the OS.
- **Teardown** (`ClosePseudoConsole` docs).
  - `ClosePseudoConsole` sends `CTRL_CLOSE_EVENT` to attached clients.
  - Before Windows 11 24H2 (build 26100) it "will wait indefinitely" unless the
    output pipe is drained or closed. So it must not be called on the thread
    that reads the output.
  - The output pipe is not guaranteed to reach EOF just because the child
    exited. It ends when the pseudoconsole is closed.
- **Exact args.**
  - `CommandBuilder` builds a single command line using MSVCRT/`ArgvQuote`
    quoting. `cmd.exe` parses differently, so passing argv exactly through
    `cmd.exe /c …` is not guaranteed.
  - The PATH search tries `PATHEXT` extensions.
  - If cwd is unset or invalid, it falls back to `%USERPROFILE%`.
- **Environment.** `get_base_env` starts from the process environment, then
  **overrides it with registry values**: HKLM `Session Manager\Environment` and
  HKCU `Environment`, with the two `PATH`s merged.
  - **Implication:** full env passthrough on Windows needs `env_clear()`
    followed by re-adding `std::env::vars_os()`.
- **Unix-only API.** `as_raw_fd`, `process_group_leader`, `tty_name` and
  `get_termios` are `#[cfg(unix)]`.
- **Windows AF_UNIX.** Stream sockets only, with no credential passing (see
  §5).

---

## Summary: gaps and workarounds

| Gap | Workaround |
|---|---|
| Signal number lost in `ExitStatus` | downcast to `std::process::Child` and use `ExitStatusExt::signal()`, or `waitpid(process_id())` |
| No cwd means `$HOME`; an invalid cwd silently becomes `$HOME` | always set `.cwd()` and validate it first |
| `SHELL` is always injected | accept and document it |
| Dropping the writer sends `\n` + EOF | keep the writer for the PTY's whole lifetime |
| `pre_exec` allocates (`read_dir`) | spawn the PTY child while the daemon is still single-threaded |
| nix 0.28 in the public API | avoid `get_termios()`, or pin a matching nix |
| Unreleased fixes on `main` | use a git dependency or `[patch]` if they are needed |
| On Windows, registry values override the env | `env_clear()`, then re-add the process env |
| ConPTY's CPR request can hang the caller | the daemon or fixture answers `ESC[6n` |
