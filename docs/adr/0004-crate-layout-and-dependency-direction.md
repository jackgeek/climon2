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
artifact, agreed in spec #73, is recorded below (#75).
The monitor is the one exception to the daemon-only `climon2-vt` rule: the
[#63 terminal contract](../protocol/terminal-integration.md) has it keep its own
`climon2-vt` models with read-only Rust views, and it already shares the
documented normal-shell baseline from that crate. This adds no cross-release
cell schema.

The workspace has one package per shipped binary. The CLI, daemon and client never link each other; they reach each other only by path at runtime (the CLI starts the daemon and execs into the version-matched client). Shared code lives in two libraries: `climon2-core`, which is platform-neutral and has no `cfg(target_os)`, and `climon2-platform`, which holds the shared seam modules (ADR 0003) and the validated client-library loader (#84). Session discovery, shared by the CLI and monitor, is a separate shipped library, `climon2-discovery` (#77). The crate boundary keeps neutral code out of the platform seams, so `core` builds and runs its tests on Linux and Windows without the rest of the workspace.

## Packages (`crates/`, plus `xtask/` at the root)

| Package | Kind | May depend on |
|---|---|---|
| `climon2-launcher` (bin `climon2`) | shipped | nothing, not even workspace crates (ADR 0002); contains seam 1 |
| `climon2-core` | shipped lib | crates.io only |
| `climon2-platform` | shipped lib | `core` |
| `climon2-cli` (bin `climon2-cli`) | shipped lib + thin main | `core`, `platform`, `discovery` |
| `climon2-discovery` | shipped lib | `core`, `platform`; one discovery pass over a climon2 home, for the CLI and the monitor |
| `climon2-daemon` (bin `climon2-daemon`) | shipped lib + thin main | `core`, `platform`, `vt` |
| `climon2-client` (bin `climon2-client`, cdylib `libclimon2_client.dylib` or `.so`) | shipped lib + thin main + cdylib | `core`, `platform` |
| `climon2-install` (bin `climon2-install`, shipped as `install`) | shipped lib + thin main | `core`; contains seam 8 |
| `climon2-monitor` (bin `climon2-monitor`) | shipped lib + thin main | `core`, `platform`, `vt`, `discovery`. Ratatui is a dependency of this package only. It never links the client or daemon packages: client libraries are reached by path at runtime |
| `climon2-vt` | shipped lib | crates.io only (the pinned `alacritty_terminal`, ADR 0007, and `unicode-width`, the engine's own width table, which monitor-model charges use to measure characters as the engine does); platform-neutral, no `cfg(target_os)`; only the daemon and the monitor may depend on it |
| `climon2-test-term`, `climon2-fixtures`, `climon2-e2e`, `xtask` | test-only, `publish = false` | anything |
| `climon2-second-release` (lib + cdylib `libclimon2_second_release`, bin `climon2-second-release-daemon`) | test-only, `publish = false`, its own version (#97) | anything; it compiles `climon2-client`'s library source and runs the `climon2-daemon` library |
| `climon2-measure` (lib + bin `climon2-measure`) | test-only, `publish = false` (#121) | anything; it consumes `core`, `platform` and `test-term`, and measures an installed candidate archive's executables without linking them |

Shipped crates may use test-only crates only as dev-dependencies. A workspace test reads `cargo metadata` to enforce these edges, and also checks that every package that owns a binary has at least one integration test.

One validated loader for client libraries lives in `climon2-platform` (`library`, #84; confirmed with the developer on 2026-11-08). The monitor and the client executable must apply exactly the same lookup, failure classification and description validation, and both already depend on `platform`, so sharing the loader adds no crate edge. It calls the system's `dlopen` through `libc`; no third-party loader crate is used. This replaces spec #73's note that a dynamic-library loader dependency belongs to the monitor only. Its Linux and macOS checks live in `platform`'s OS-specific files, and the native `#[repr(C)]` declarations it validates stay in `core`.

The client library is an artifact of `climon2-client`, not another crate: its library target is `crate-type = ["lib", "cdylib"]`, producing `libclimon2_client.dylib` on macOS and `libclimon2_client.so` on x86-64 GNU/Linux for every version folder. `climon2_core::layout::client_library_name` derives that file name from the target's dynamic-library prefix and suffix; the installer and xtask take it from there. `climon2_core::layout::ExecutableHome`, an absolute, canonical executable home, owns each version's library path, `<home>/<version>/<library file>`, as `ExecutableLayout` owns version binaries' paths. It exports only the frozen `climon2_client_get_interface` bootstrap of the native client interface; Rust layouts, unwinding and allocators never cross it. The `#[repr(C)]` declarations of that interface live in `climon2-core` (`native`), so the library, the client executable and the monitor can share them without new crate edges; the monitor reaches libraries only by path at runtime and never depends on `climon2-client`.

The [second-release fixture](../../GLOSSARY.md#testing) (#97; accepted by the developer on 2026-11-15, [decision on #97](https://github.com/jackgeek/climon2/issues/97#issuecomment-6892644920), and extended to a session daemon the same day, [daemon decision on #97](https://github.com/jackgeek/climon2/issues/97#issuecomment-6892729286)) is the one package with a version of its own. Mixed-release tests need a second release whose client library and session daemon each have their own release identity and deliberately different private behaviour, built by the same Cargo run, without nested builds. `climon2-second-release` compiles `climon2-client`'s unchanged `library` source through an inline `#[path]` module, so the library's release identity is its own package version, `0.0.97`; it never enables that source's `test-hooks` faults. Its `cdylib` exports only the bootstrap and passes the release checks as 0.0.97's library. Its `lib` crate type exists only so that `climon2-e2e` and `xtask`, which consume it as dev-dependencies, make Cargo build the `cdylib` into `deps/`; nothing links it. Its `climon2-second-release-daemon` binary runs the unchanged `climon2-daemon` library behind a thin main of its own. The daemon library takes its release from its invocation, so the main compiles the identity in: it refuses an invocation naming any release but `0.0.97`. It also gives its session's command `CLIMON2_SECOND_RELEASE_DAEMON`, a private behaviour that a session's title makes visible over revision 1 while the frozen hello and revision 1 stay unchanged. The package's integration test makes `cargo test` build that binary. Neither artifact is ever shipped: release builds name shipped packages with `-p`, and the installer's payload has no place for them. CRAP and mutation already cover the shared library and daemon source through their own packages.

## Considered options

- **One `climon2` package with three bins:** rejected. Each binary keeps its own dependencies, and the client library mustn't grow code that only the daemon needs. A future monitor loads version-matched client libraries by path, using a separately specified embedding interface rather than a Cargo dependency on another release.
- **One shared library containing both the core and platform modules:** rejected. `compile_error!` would stop the neutral code building on Linux and Windows.
- **Building the CLI as `climon2` and renaming the launcher:** rejected. The CLI is `climon2-cli` everywhere, so the launcher alone is `climon2`, and Cargo binary names don't collide.
- **The client-library loader in the monitor only, or a third-party loader crate:** rejected (#84). The client executable needs the same checks, and two loaders would drift apart; a few `libc` calls do the loading, and the classification and validation rules are climon2's own.
- **The monitor inside the CLI package or binary:** rejected. Ratatui, the monitor's `climon2-vt` models and its library loading would join every CLI invocation's dependencies; a separate executable keeps them in the monitor package alone.
- **The second-release fixture as a C stand-in, a patched copy of the real library, or a nested build (#97):** rejected. A C stand-in would implement revision 1 again and could not open a passive connection to a real session daemon; patching the release text into a built library forges an identity; a nested Cargo build overwrote feature-unified artifacts (#31). Compiling the daemon library itself under the fixture's version would not give the daemon an identity, because the daemon takes its release from its invocation; a thin main gives it one without changing shipped source.
- **Discovery staying in the CLI package:** rejected. The monitor would have to link the CLI or copy its discovery; one library keeps a single implementation of metadata validation, probing and the staleness rules.

## Consequences

- Plain `cargo test` builds a package's real binaries only if that package has an integration test, and `CARGO_BIN_EXE_*` only covers binaries in the same package. Tests therefore find binaries from other packages as siblings of `current_exe().parent()`. Nested `cargo build` is ruled out: it silently overwrote feature-unified binaries.
- `test-hooks` is turned on only through `climon2-test-term`'s normal dependencies, so the workspace-wide `cargo test` builds hooked binaries. A package without a `climon2-test-term` dev-dependency, such as the monitor, gets its plain binary from `cargo test -p` alone, at the same sibling path; `cargo xtask test` runs the monitor's build test that way before the workspace tests, so every gate finds both. Release builds must use `-p <package>` for each shipped package, never `--workspace`, and a release gate checks that no hook marker is present in the shipped binaries.
- Cargo builds the client library's cdylib whenever the `climon2-client` library is compiled, but uplifts only binaries and directly requested targets. Under `cargo test` the hooked library therefore stays beside the test executables in `deps/`, where tests and `Payload::from_siblings` find it without nested builds; per-package release builds uplift the unhooked library. Its crate type keeps `lib` so that the package still owns a library target. Unlike the monitor's binary, the library has no plain `cargo test` build: `climon2-client`'s tests enable `test-hooks` through their `climon2-test-term` dev-dependency, and Cargo names a cdylib target's outputs without a hash, so another package's plain test build would replace the hooked library at the same path. The plain library is the per-package release build, which `cargo xtask release-check` builds in a private target directory and checks.
- On x86-64 GNU/Linux the `climon2-client` library target links the GCC unwinder (`gcc_eh`) statically, so the cdylib needs no shared library beyond glibc's, as library embedding requires; no unwinding crosses its C ABI. Every artifact that links the crate, including the client executable and its tests, therefore also stops needing `libgcc_s.so.1`.
- The `climon2-client` library target declares the `#[global_allocator]` that measures each instance's allocation (#180). It replaces the allocator of every artifact that links the crate: the client library, the client executable and the package's own test binaries; each block carries a 16-byte prefix (or one alignment's worth), and nothing is charged outside an instance. Packages that only depend on `climon2-client` to build it, such as `climon2-test-term` for `test-hooks`, never reference it, so their test binaries do not link it.
- CRAP and mutation gates run on the shipped packages only.
