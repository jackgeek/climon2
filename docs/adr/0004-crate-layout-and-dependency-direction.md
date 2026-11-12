# Crate layout and dependency direction

**Monitor:** the [monitor ownership decision](https://github.com/jackgeek/climon2/issues/58)
requires directly loaded client libraries and client threads in the monitor.
The [discovery decision](https://github.com/jackgeek/climon2/issues/59) planned a
discovery crate shared by CLI and monitor. #77 moved the CLI's session discovery
into it: `climon2-discovery`, a shipped library that the CLI uses now and the
monitor will use (#106). The monitor is its own shipped
package and executable, `climon2-monitor` (spec #73, confirmed 2026-11-08): the
CLI execs its own version's monitor for `c2 monitor` and `c2 mon` (or their
`climon2` spellings), as it execs the version-matched client, and that handoff
between executables of one release is not frozen. The client library's `cdylib`
artifact, agreed in spec #73, is recorded here once it is built (#75).
The monitor is the one exception to the daemon-only `climon2-vt` rule: the
[#63 terminal contract](../protocol/terminal-integration.md) has it keep its own
`climon2-vt` models with read-only Rust views, and it already shares the
documented normal-shell baseline from that crate. This adds no cross-release
cell schema.

The workspace has one package per shipped binary. The CLI, daemon, client and monitor never link each other; they reach each other only by path at runtime (the CLI starts the daemon and execs into the version-matched client or its own version's monitor). Shared code lives in two libraries: `climon2-core`, which is platform-neutral and has no `cfg(target_os)`, and `climon2-platform`, which holds the shared seam modules (ADR 0003). The crate boundary keeps neutral code out of the seams, so `core` builds and runs its tests on Linux and Windows without the rest of the workspace. Session discovery, shared by the CLI and the monitor, is a third library, `climon2-discovery`, built on those two.

## Packages (`crates/`, plus `xtask/` at the root)

| Package | Kind | May depend on |
|---|---|---|
| `climon2-launcher` (bin `climon2`) | shipped | nothing, not even workspace crates (ADR 0002); contains seam 1 |
| `climon2-core` | shipped lib | crates.io only |
| `climon2-platform` | shipped lib | `core` |
| `climon2-cli` (bin `climon2-cli`) | shipped lib + thin main | `core`, `platform`, `discovery` |
| `climon2-discovery` | shipped lib | `core`, `platform`; one discovery pass over a climon2 home, for the CLI and the monitor |
| `climon2-daemon` (bin `climon2-daemon`) | shipped lib + thin main | `core`, `platform`, `vt` |
| `climon2-client` (bin `climon2-client`) | shipped lib + thin main | `core`, `platform` |
| `climon2-install` (bin `climon2-install`, shipped as `install`) | shipped lib + thin main | `core`; contains seam 8 |
| `climon2-monitor` (bin `climon2-monitor`) | shipped lib + thin main | `core`, `platform`, `vt`, `discovery`. Ratatui is a dependency of this package only. It never links the client or daemon packages: client libraries are reached by path at runtime |
| `climon2-vt` | shipped lib | crates.io only (the pinned `alacritty_terminal`, ADR 0007); platform-neutral, no `cfg(target_os)`; only the daemon and the monitor may depend on it |
| `climon2-test-term`, `climon2-fixtures`, `climon2-e2e`, `xtask` | test-only, `publish = false` | anything |

Shipped crates may use test-only crates only as dev-dependencies. A workspace test reads `cargo metadata` to enforce these edges, and also checks that every package that owns a binary has at least one integration test.

## Considered options

- **One `climon2` package with three bins:** rejected. Each binary keeps its own dependencies, and the client library mustn't grow code that only the daemon needs. A future monitor loads version-matched client libraries by path, using a separately specified embedding interface rather than a Cargo dependency on another release.
- **One shared library containing both the core and platform modules:** rejected. `compile_error!` would stop the neutral code building on Linux and Windows.
- **Building the CLI as `climon2` and renaming the launcher:** rejected. The CLI is `climon2-cli` everywhere, so the launcher alone is `climon2`, and Cargo binary names don't collide.
- **The monitor inside the CLI package or binary:** rejected. Ratatui, the monitor's `climon2-vt` models and its library loading would join every CLI invocation's dependencies; a separate executable keeps them in the monitor package alone.
- **Discovery staying in the CLI package:** rejected. The monitor would have to link the CLI or copy its discovery; one library keeps a single implementation of metadata validation, probing and the staleness rules.

## Consequences

- Plain `cargo test` builds a package's real binaries only if that package has an integration test, and `CARGO_BIN_EXE_*` only covers binaries in the same package. Tests therefore find binaries from other packages as siblings of `current_exe().parent()`. Nested `cargo build` is ruled out: it silently overwrote feature-unified binaries.
- `test-hooks` is turned on only through `climon2-test-term`'s normal dependencies, so the workspace-wide `cargo test` builds hooked binaries. A package without a `climon2-test-term` dev-dependency, such as the monitor, gets its plain binary from `cargo test -p` alone, at the same sibling path; `cargo xtask test` runs the monitor's build test that way before the workspace tests, so every gate finds both. Release builds must use `-p <package>` for each shipped package, never `--workspace`, and a release gate checks that no hook marker is present in the shipped binaries.
- CRAP and mutation gates run on the shipped packages only.
