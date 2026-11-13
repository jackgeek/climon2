# Executables and dynamic libraries

This diagram covers every climon2-owned executable target and dynamic library,
including development fixtures. Arrows run from consumer to consumed artifact;
labels distinguish execution, communication, and loading.
Solid relationships exist today; dashed relationships are planned.
The current executable relationships apply on macOS and x86_64 GNU/Linux;
native archives are described in [distribution](distribution.md).
The development host owns shared/Linux CRAP and mutation; CI qualifies only
macOS-exclusive source, while retaining native runtime/archive tests. This
[quality split](distribution.md#release-gates) changes no artifact or
consumption relationship.

```mermaid
flowchart TB
    subgraph runtime["Shipped runtime"]
        launcher["c2 / climon2<br/>same launcher executable"]
        installer["install<br/>built as climon2-install"]
        subgraph version["Installed version v; versions coexist"]
            cli["climon2-cli<br/>CLI executable"]
            client["climon2-client<br/>client executable"]
            daemon["climon2-daemon<br/>session daemon executable"]
            monitor["climon2-monitor<br/>monitor executable; Ratatui TUI"]
            library[["libclimon2_client.dylib / .so<br/>Rust cdylib; C ABI bootstrap"]]
        end
        command(["Session command<br/>external executable"])
        source_monitor(["Source monitor runtime<br/>future forwarding provider"])
        browser(["Browser with xterm.js<br/>future external consumer"])

        launcher -->|"execs current version"| cli
        cli -->|"starts for exec"| daemon
        cli -->|"exec / attach / kill: session version"| client
        cli -->|"hello probes for discovery"| daemon
        client -->|"daemon-client communication; current"| daemon
        daemon -->|"runs on session PTY"| command
        cli -->|"monitor / mon: execs own version"| monitor
        monitor -.->|"hello probes for discovery; planned"| daemon
        monitor -.->|"loads session-version library through the shared loader; one client thread per live session; planned"| library
        client -.->|"loads same-release library through the shared loader; versioned C interface planned"| library
        library -.->|"daemon-client communication; planned"| daemon
        monitor -.->|"consumes forwarded sessions over TCP/IP; future"| source_monitor
        browser -.->|"consumes forwarded terminal traffic over WebSocket; future"| monitor
    end

    subgraph development["Development and test-only executables"]
        xtask["xtask"]
        cargo(["Cargo build/test tooling<br/>external"])
        tests(["Cargo test harnesses<br/>generated executables"])
        cli_fixture["climon2-cli-fixture"]
        daemon_fixture["climon2-daemon-fixture"]
        terminal_fixture["climon2-fixture"]
        tui_fixture["climon2-tui-fixture"]
        library_host["climon2-library-host"]
        library_stand_in[["Client library stand-in<br/>test-only C dylib / .so; faulty library"]]
        owned_exec["climon2-test-owned<br/>Linux test ownership exec adapter"]
        terminal_root(["Requested test terminal root<br/>climon2 executable, fixture or external program"])

        xtask -->|"runs builds and tests"| cargo
        xtask -->|"install: runs verified production payload"| installer
        xtask -->|"runs runtime probes"| cli
        xtask -->|"loads through the shared loader in release checks"| library
        cargo -->|"runs"| tests
        tests -->|"uses directly or as a session command"| cli_fixture
        tests -->|"uses directly or as a session command"| daemon_fixture
        tests -->|"uses directly or as a session command"| terminal_fixture
        tests -->|"uses directly or as a session command"| tui_fixture
        tests -->|"runs as the Library-layer host"| library_host
        library_host -->|"loads through the shared loader; calls through the C ABI"| library
        tests -->|"loads through the shared loader in release-check and Fault-layer tests"| library_stand_in
        tests -->|"execs registered terminal roots through; Linux only"| owned_exec
        owned_exec -->|"execs with unchanged argv, environment and cwd"| terminal_root
    end

    style source_monitor stroke-dasharray: 5 5
    style browser stroke-dasharray: 5 5
```

`v` represents a [session](../GLOSSARY.md#sessions)'s installed version, not necessarily the current
version selected by the launcher. The CLI starts a new session using its own
version; [attachment](../GLOSSARY.md#attachment-and-control) and termination use the existing session's version.
The installed `c2` command is a relative symlink to `climon2` in the same
executable home. Both names consume the same current-version CLI through the
unchanged launcher; `climon2` remains supported without deprecation warnings.
The installer is a release-payload executable, not a `c2 install` command.
Installation relationships are outside this consumption diagram.

For workspace crate consumption, see the [crate diagram](crate-relationships.md).

On Linux, `climon2-test-term` builds `climon2-test-owned` as test tooling, never
as a release payload. It opens one test-owned file descriptor before execing
the requested terminal root. Forked descendants inherit that descriptor across
environment clearing and reparenting, so procfs leak checks do not depend on
catching short-lived parents. Process identity, sampled ancestry, session
membership and environment markers remain additional ownership evidence.
Commands that explicitly close inherited descriptors cannot retain this
additional evidence. macOS keeps its kernel process/image ancestry tracking
and does not use this adapter.

**Current (#75):** every release builds and ships the client library in its
version folder: `libclimon2_client.dylib` (universal arm64/x86_64) on macOS and
`libclimon2_client.so` on x86-64 GNU/Linux, both from one Rust cdylib artifact
of `climon2-client`. Each exports only the frozen
`climon2_client_get_interface` bootstrap and implements the bootstrap and
description of [native revision 1](protocol/monitor-client/core-v1.md); its
instance calls report unsupported operation. The Linux library links the GCC
unwinder statically, because no unwinding leaves it, so it needs only glibc's
libraries and has no soname, RPATH or RUNPATH, as the
[Linux decision](https://github.com/jackgeek/climon2/issues/125) requires for
release isolation; release checks enforce each rule. Only test tooling loads it
today, the library host fixture and xtask's release checks; the monitor's and
client executable's loading remain planned. Cargo's ordinary
`lib` targets, including `climon2-core`, `climon2-platform`, and `climon2-vt`,
are not separately loaded dynamic libraries. System and third-party libraries
and individual generated test harness names are outside this artifact inventory.

**Planned:** the [monitor](../GLOSSARY.md#sessions) loads version-matched
[client libraries](../GLOSSARY.md#sessions) directly into
its own process, sharing a loaded library for sessions on the same release.
Each live session has its own client thread. Clients start eagerly, newest
first, with bounded concurrent progress; continuous discovery adds clients for
new sessions. Background clients relay negotiated plugin data without taking
control. Native client crashes can terminate the entire monitor. Ratatui powers
the monitor's rendering; today it draws only the permanent top row and the empty
state, and the rest of its terminal integration is agreed, unimplemented.
The [agreed terminal contract](protocol/terminal-integration.md), not implemented,
selects exact ordered terminal bytes/control/restoration through existing
in-process record queues and monitor-owned `climon2-vt` models, with planned
read-only Rust views for Ratatui. Session bytes never go directly to the host;
daemon query authority stays unique. Monitor-mode daemon presentation uses
effective session geometry, not host/requested dimensions, including background
promotion. Optional core Focus is not a plugin/artifact addition. No new dynamic
library is chosen by this amendment.

The [status/activity revision](session-status.md) puts title detection and both
state machines and terminal-signal interpretation in
[daemon](../GLOSSARY.md#sessions) core, not plugins.
The client library carries core title/status reports, structured notification/
progress metadata, and explicit [viewing](../GLOSSARY.md#status-and-attention) information. Metadata is available
without control, not raw [OSC](protocol/TERMS.md#osc) passthrough to host terminals. Root program status
and shell lifecycle choose explicit detection or heuristic fallback.
Plugin implementation, configuration, and interface examples are deferred; the earlier title-plugin
release prerequisite is superseded. No new plugin executable or dynamic library
is specified. These are planned mechanics, not shipped behavior.

The monitor retains a `climon2-vt` model per session for disconnected terminal/
scrollback inspection and reconnects passively without automatic takeover.
Known completion stops retries. These relationships do not introduce a session
survival guarantee or a persisted archive of completed sessions.
The [agreed lifecycle](monitor-lifecycle.md), not implemented, adds exponential
passive recovery, read-only disconnected history with a `Reconnecting...` overlay,
and coordinated concurrent shutdown. Local instance cleanup and joins precede
run-local selector dismissal; stalled cleanup ends the whole monitor process,
never an individual thread or loaded library. Connected revoked sessions show
their placeholder, not cached history. Selector Delete uses the existing Kill
semantics and adds confirmed termination/removal, not another runtime artifact.

The agreed [operational limits](monitor-operational-limits.md), unimplemented,
bound admission, queues, client threads/workers/descriptors and retained shared
release resources. Resource-blocked rows remain visible; history pressure
preserves a recent page. Unsafe growth/cleanup is explicit failure, not client
eviction or unloading. Allocation-aware terminal qualification reopens the
narrow unbounded-engine assumption; it chooses no new executable/library or
consumption arrow, and does not sandbox trusted native code.

The client executable will also load its same-release library for ordinary
operations.

**Current (#84):** one validated loader for client libraries is part of
`climon2-platform`, not a separate artifact; the monitor and the client
executable will both load through it. Today the library host and xtask's
release checks load the client library through it, from a version folder of
an executable home. The [client library stand-in](../GLOSSARY.md#testing),
`crates/climon2-fixtures/stand-in/client-library.c`, is compiled by tests into
a test-only library, exact or with one defect: xtask's release-check tests
load it through the same release checks as the real library, and
`climon2-platform`'s loader tests load it, as the faulty library, through the
loader. It is never shipped.

**Current (#74):** every version folder ships the `climon2-monitor` executable,
on macOS and x86-64 GNU/Linux. `c2 monitor` and `c2 mon`, or their `climon2`
spellings, reach it through the launcher and the current version's CLI, which
execs its own version's monitor exactly as it execs a version-matched client,
so the monitor is always the current version. That handoff between executables
of one release is not frozen. The monitor does not yet discover sessions or
load libraries.

The agreed [embedding design](protocol/library-embedding.md) loads
`<executable home>/<session version>/libclimon2_client.dylib` on macOS, or
`libclimon2_client.so` on x86-64 GNU/Linux, with no release fallback. It uses a versioned C interface, per-instance client threads,
release-private dependency resolution, and explicit memory ownership.
Every successfully opened library handle stays loaded until monitor exit,
including handles rejected by later validation. Such releases remain in use
until process exit; completed instance cleanup and joins do not unload them.
These loading relationships are planned; only the artifacts ship today. Complete interface
declarations are specified in [native revision 1](protocol/monitor-client/core-v1.md),
with explicit startup/role, ownership, cleanup and fresh-instance recovery calls.
No new artifact or consumption arrow is added. Monitor-client communication is
an agreed specification, not implemented.
[Daemon wire revision 1](protocol/daemon-client/core-v1.md) is agreed and
partly implemented: session daemons serve
[passive monitor connections](../GLOSSARY.md#attachment-and-control) over it,
beside the private protocol, but no climon2 executable or library consumes it
yet, so no arrow changes.

The planned wire core is independent of the current private Unix socket carrier.
The main/downstream monitor consumes source-monitor sessions over
[TCP/IP](protocol/TERMS.md#tcpip) and presents them as local sessions.
A browser consumes that monitor's forwarded terminal traffic over
[WebSocket](protocol/TERMS.md#websocket) into xterm.js. Dashed arrows above point
from consumer to provider, not in the direction [bytes](protocol/TERMS.md#byte) happen to flow.
No native client library is assumed inside a browser. Routing, multiplexing,
discovery and remote authentication/security remain deliberately unresolved;
same-[UID](protocol/TERMS.md#uid)/private-directory local checks do not define
future transport security. No forwarding implementation is claimed.

Future `c2 exec --headless` returns after daemon-confirmed creation without
starting an interactive client. A [headless session](../GLOSSARY.md#sessions)
keeps its explicit zero-client survival policy throughout its lifetime, including
after later clients unexpectedly leave; command completion or existing CLI Kill
ends it. The [lifecycle decision](monitor-lifecycle.md) agrees this narrow
exception, not universal persistence or a new executable/library. Implementation
remains unshipped; the current execution arrows above are unchanged.

The planned relationships are indexed by
[Chart the interactive monitor specification](https://github.com/jackgeek/climon2/issues/57);
the hosting and control decision lives in
[Define monitor client ownership and control](https://github.com/jackgeek/climon2/issues/58).
The diagram is a relationship index, not a second interface specification.
