# climon2-monitor-tui

The [monitor](../../GLOSSARY.md#sessions)'s terminal interface. It builds the
`climon2-monitor` executable that `c2 monitor` and `c2 m` start, and is the
one consumer of the [monitor library](../climon2-monitor/README.md) today
([ADR 0011](../../docs/adr/0011-monitor-library-and-client-embedding.md),
#490). Its library boundary is `run() -> Result<u32, Error>`; the thin main
prints `climon2: error: …` for an `Error` and exits 125.

It holds what belongs to a terminal and to no other device: checking that
stdin and stdout are terminals, entering and releasing the host, decoding the
host's input (the client escape prefix, key bindings, SGR mouse reports,
bracketed pastes and focus reports) into the library's actions, the
selector's navigation, the key introduction and its preference file, where a
toast sits and what it covers, painting with Ratatui at most `ui.fps` frames
a second, writing the host attention reports and the bell, the exit status,
and the fail-stop when the library's verdict demands it.

It depends on `climon2-core`, `climon2-platform` and `climon2-monitor`. It
reaches each session's terminal model only through the read-only views the
library re-exports; only its tests build models of their own, through a `dev`
dependency on `climon2-vt`. Ratatui is a dependency of this crate alone.

How the monitor behaves, which module of which crate holds each part, and the
tests that prove each scenario are in the
[monitor library's README](../climon2-monitor/README.md).
