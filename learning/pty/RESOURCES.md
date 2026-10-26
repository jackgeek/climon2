# PTY Session Shadowing Resources

## Knowledge

- [Linux manual: `pty(7)`](https://man7.org/linux/man-pages/man7/pty.7.html)
  Primary description of the Unix master/slave PTY pair and its bidirectional byte flow. Use for: the core PTY mental model and Linux-specific details.
- [Linux manual: `openpty(3)` / `forkpty(3)`](https://man7.org/linux/man-pages/man3/openpty.3.html)
  Primary description of allocating a PTY, creating a session, and making the slave the controlling terminal. Use for: process setup and lifecycle.
- [The Open Group: `posix_openpt`](https://pubs.opengroup.org/onlinepubs/9699919799/functions/posix_openpt.html)
  Standardized Unix API for opening a PTY master. Use for: separating portable POSIX behavior from platform conveniences.
- [Microsoft: Pseudoconsoles](https://learn.microsoft.com/en-us/windows/console/pseudoconsoles)
  Official overview of ConPTY as a host for Windows character-mode applications with UTF-8 terminal channels. Use for: Unix/Windows architectural comparisons.
- [Microsoft: Creating a Pseudoconsole session](https://learn.microsoft.com/en-us/windows/console/creating-a-pseudoconsole-session)
  Official lifecycle and pipe setup for ConPTY, including threading and deadlock warnings. Use for: Windows implementation details.
- [`nix::pty` Rust documentation](https://docs.rs/nix/latest/nix/pty/index.html)
  Rust wrappers around Unix PTY operations such as `openpty` and `forkpty`. Use for: hands-on macOS/Linux exercises.
- [The Rust Programming Language: Ownership](https://doc.rust-lang.org/book/ch04-01-what-is-ownership.html)
  Official introduction to Rust ownership. Use for: reasoning about which component owns the PTY master and session lifetime.

## Wisdom (Communities)

- [Rust Users Forum](https://users.rust-lang.org/)
  Moderated technical community with strong systems-programming expertise. Use for: reviewing Rust API and ownership decisions.
- [Unix & Linux Stack Exchange](https://unix.stackexchange.com/)
  High-signal Q&A on terminal sessions, job control, and Unix process behavior. Use for: testing platform assumptions against practitioner experience.

## Gaps

- Add an authoritative Darwin/macOS PTY source when a stable Apple-hosted reference is identified.
- Add a primary source for terminal escape-sequence parsing before implementing screen-state reconstruction.
