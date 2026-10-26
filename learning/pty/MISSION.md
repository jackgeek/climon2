# Mission: PTY Session Shadowing in Rust

## Why
Build a terminal-sharing feature that can run a real shell and let another client shadow its session. Learn Rust through that project rather than as a separate prerequisite.

## Success looks like
- Explain where a PTY ends and session shadowing begins
- Build a Rust host that runs a shell behind a PTY on macOS
- Attach and detach read-only and interactive viewers safely
- Handle terminal resize, process exit, backpressure, and input ownership
- Port the design to Linux and explain how ConPTY changes the platform boundary

## Constraints
- The learner is new to both PTYs and Rust, but comfortable with processes
- Rust concepts should be introduced just in time
- Hands-on work targets macOS first, with Linux differences called out
- Windows ConPTY should be compared at equivalent architectural boundaries

## Out of scope
- Building a full terminal emulator or VT parser before shadowing works
- Production authentication, encryption, or internet transport in the first phase
- Mastering Rust independently of the terminal-sharing project
