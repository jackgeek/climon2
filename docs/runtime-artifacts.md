# Executables and dynamic libraries

This diagram covers every climon2-owned executable target and dynamic library,
including development fixtures. Arrows run from consumer to consumed artifact;
labels distinguish execution, communication, loading, and installation.
Solid relationships exist today; dashed relationships are planned.

```mermaid
flowchart TB
    subgraph runtime["Shipped runtime"]
        launcher["climon2<br/>launcher executable"]
        installer["install<br/>built as climon2-install"]
        subgraph version["Installed version v; versions coexist"]
            cli["climon2-cli<br/>CLI executable"]
            client["climon2-client<br/>client executable / planned helper"]
            daemon["climon2-daemon<br/>session daemon executable"]
            library[["Client dynamic library<br/>planned Rust cdylib; filename undecided"]]
        end
        command(["Session command<br/>external executable"])
        monitor(["Monitor runtime: monitor / mon<br/>planned; executable placement undecided"])

        installer -->|"installs"| launcher
        installer -->|"installs"| cli
        installer -->|"installs"| client
        installer -->|"installs"| daemon
        installer -.->|"installs; planned"| library
        launcher -->|"execs current version"| cli
        cli -->|"starts for exec"| daemon
        cli -->|"exec / attach / kill: session version"| client
        cli -->|"hello probes for discovery"| daemon
        client -->|"daemon-client communication; current"| daemon
        daemon -->|"runs on session PTY"| command
        cli -.->|"monitor / mon; planned"| monitor
        monitor -.->|"starts and communicates with one session-version helper per visited session"| client
        client -.->|"loads same-version library through C-compatible interface"| library
        library -.->|"daemon-client communication; planned"| daemon
    end

    subgraph development["Development and test-only executables"]
        xtask["xtask"]
        cargo(["Cargo build/test tooling<br/>external"])
        tests(["Cargo test harnesses<br/>generated executables"])
        cli_fixture["climon2-cli-fixture"]
        daemon_fixture["climon2-daemon-fixture"]
        terminal_fixture["climon2-fixture"]
        tui_fixture["climon2-tui-fixture"]

        xtask -->|"runs builds and tests"| cargo
        xtask -->|"runs runtime probes"| cli
        cargo -->|"runs"| tests
        tests -->|"uses directly or as a session command"| cli_fixture
        tests -->|"uses directly or as a session command"| daemon_fixture
        tests -->|"uses directly or as a session command"| terminal_fixture
        tests -->|"uses directly or as a session command"| tui_fixture
    end

    style library stroke-dasharray: 5 5
    style monitor stroke-dasharray: 5 5
```

`v` represents a session's installed version, not necessarily the current
version selected by the launcher. The CLI starts a new session using its own
version; attachment and termination use the existing session's version.
The installer is a release-payload executable, not a `climon2 install` command.

**Current:** no climon2 dynamic library is built or shipped. Cargo's ordinary
`lib` targets, including `climon2-core`, `climon2-platform`, and `climon2-vt`,
are not separately loaded dynamic libraries. System and third-party libraries
and individual generated test harness names are outside this artifact inventory.

**Planned:** the client executable hosts its same-version dynamic library in
one isolated helper process per visited session. The monitor does not load
session client libraries itself. Its node represents a runtime role, not a
decision to ship an executable named `climon2-monitor`. Library filenames,
loading/lifetime details, monitor-helper communication, and the frozen
daemon-client contract remain specification work.

The planned relationships are indexed by
[Chart the interactive monitor specification](https://github.com/jackgeek/climon2/issues/57);
the hosting and control decision lives in
[Define monitor client ownership and control](https://github.com/jackgeek/climon2/issues/58).
The diagram is a relationship index, not a second interface specification.
