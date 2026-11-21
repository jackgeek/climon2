# Release distribution

## Supported artifacts

| Platform | Release archive | Build host |
|---|---|---|
| macOS 15+, Apple Silicon and Intel | `climon2-<version>-macos-universal.tar.gz` | arm64 macOS, with both Darwin Rust targets |
| GNU/Linux x86_64, qualified on Ubuntu 24.04 | `climon2-<version>-linux-x86_64-gnu.tar.gz` | x86_64 GNU/Linux |

Published Linux releases are dynamically linked GNU libc executables, built on
Ubuntu 24.04 with a supported runtime baseline of GNU libc 2.39 or newer;
musl, other architectures and older GNU libc baselines are not qualified.
Local distribution builds inherit their build host's libc baseline. A native
Arch Linux build is useful runtime evidence, not proof of the published
Ubuntu baseline or compatibility with every Linux distribution.
The Linux installer requires a filesystem supporting `renameat2` directory
exchange. macOS executables retain universal slices and ad-hoc signing.

`cargo xtask dist` builds only the shipped packages, in a private target
directory, without test hooks or inherited instrumentation. It checks release
versions and tags, verifies every executable's and the client library's
architecture (and macOS signatures), and rejects hook markers. The client
library must export only `climon2_client_get_interface` and describe exactly
the release version; on Linux it must also have no soname, RPATH or RUNPATH and
need no shared library beyond glibc's. Outputs go to `target/dist`:

- The native platform's versioned archive.
- `install.sh`, stamped with that exact version.
- `SHA256SUMS`, covering both files.

Each archive contains the installer `install`, launcher `c2`, license,
README and `<version>/{climon2-cli,climon2-daemon,climon2-client,climon2-monitor}`
plus the client library, `libclimon2_client.dylib` on macOS or
`libclimon2_client.so` on Linux.
There are no test fixtures or Cargo dependencies inside the archive.

## Installation

The published install script detects macOS or x86_64 GNU/Linux before
downloading. It verifies the selected archive with `shasum` on macOS or
`sha256sum` on Linux, extracts it, and runs its real installer. Quarantine
removal is macOS-only. Both platforms default to `$HOME/.local/share/climon2`;
`--dir` overrides the executable home without changing shell profiles.

Fresh installs and upgrades place the launcher in that home as `c2`, the only
command name; there is no `climon2` command. Every install replaces `c2`
atomically. Nothing is migrated: a `c2 -> climon2` symlink left by a development
install from before #303 is replaced by the launcher, and the `climon2` file it
pointed to stays until you delete it.

For manual Linux installation:

```sh
sha256sum -c SHA256SUMS
tar -xzf climon2-0.1.0-linux-x86_64-gnu.tar.gz
./climon2-0.1.0-linux-x86_64-gnu/install
```

Download all files listed by `SHA256SUMS` before checking the entire manifest;
alternatively select the checksum line for the downloaded archive.

## Release gates

The checks workflow runs full runtime/terminal tests and strict formatting,
Clippy and ShellCheck on native macOS arm64, macOS x86_64 and Linux x86_64.
Shared and Linux-specific coverage-backed CRAP and mutation, including
instrumentation/worker-isolation proofs, run on the dedicated Linux development
host. CI does not repeat them. macOS arm64 CI runs
`cargo xtask mutate --native-only`, scoring only registered macOS-exclusive
source and mutating changed files in that set. Shared-only changes skip this
quality run. The native-only run still collects real workspace coverage and
requires the unchanged full-workspace worker baseline before mutation.
Both scopes keep CRAP <= 5 and zero surviving mutants.
Windows retains its platform-neutral contract job.

Local qualification measures mutation load before mutation, even when no
changed source needs mutation. The ticket gate (`cargo xtask check`) runs one
round and mutates only added or changed functions; the full gate
(`cargo xtask check --full`, run before a feature branch merges to `main`)
runs five rounds and mutates whole changed files. First, unloaded in the warm
mutation build, each target's earlier mutant stages (library tests, then its
crate's integration tests) are timed once; each stage timeout is 10 times its
run, at least 2 s. After timing one unloaded full-workspace run, each round
runs 8 suites concurrently, each in a private warm-build copy and its own
session. Each suite rehearses a mutant's run on the unchanged workspace: the
earlier stages in order under their stage timeouts, then the full workspace,
all within the mutator's timeout (10 times the unloaded full-workspace run).
Zero failures are required, and no stage may outlive its timeout: the mutator
counts either as a killed mutant, which could hide a survivor. No test is
retried. A failing
round stops qualification and names each failing test and test target with its
failed-suite count. Every run's log remains under `target/flake-logs/` in a
unique directory. `cargo xtask mutate`, including macOS native-only CI, does
not measure load.

Native distribution jobs produce separate artifacts. Archive jobs verify
checksums, exercise the download script against those exact files, and run
packaged acceptance including bash smoke, vim and less. macOS runs the same
universal archive on both architectures; Linux runs its own ELF archive.
`CLIMON2_E2E_ARCHIVE` selects the exact archive, without binary substitutions:

```sh
CLIMON2_E2E_ARCHIVE="$PWD/target/dist/climon2-0.1.0-linux-x86_64-gnu.tar.gz" \
  cargo test --workspace --locked
```

Previous-release compatibility uses the most recent published archive for
the same platform. The first Linux release does not pretend a macOS archive
is a prior Linux release. Only an explicitly absent previous platform archive
permits the first-platform-release exception.

Before tagging, the maintainer must run `cargo xtask check --full --mutate-all` locally
for the exact release commit. CI cannot certify that local result. Tagged
releases require all macOS-exclusive source to pass CRAP and mutation in CI,
plus every native runtime and archive job.

### Release qualification

Between building and publishing, the Release workflow qualifies the exact
candidate archives against the [acceptance matrix](monitor-acceptance.md)
([#122](https://github.com/jackgeek/climon2/issues/122)). Its `qualify` jobs
run natively on `ubuntu-24.04` (x86-64 GNU/Linux), `macos-15` (arm64) and
`macos-15-intel` (x86_64), always, whatever `CLIMON2_MACOS_TESTS` says (owner
decision on #122); pull requests never qualify. Each job runs
`cargo xtask qualify run`, which records every scenario case, each scenario's
required layer on that platform, in one of the states **not implemented**,
**not run**, **passed**, **failed** or **blocked**:

- Contract, Fault and Docs cases come from the tests the crate READMEs'
  scenario-to-test tables name: the packaged tests against the exact archive
  (`CLIMON2_E2E_ARCHIVE`), and every workspace test from the candidate's
  source revision on hook-enabled debug builds. A pure supplement proves no
  layer. A test that is skipped (ignored) blocks its case.
- Library and Monitor cases count only the packaged tests against the
  archive. A packaged test that returns at once when given an archive (it
  needs test hooks) proves nothing there.
- Measure cases (NOT-02, STA-02, LIM-06 and PERF-01) come from this runner's
  [measurement job](../GLOSSARY.md#testing) reports for all three
  geometries. A report of another archive or a reduced window blocks them; a
  missed target or requirement fails them.
- The Host manual case (FOC-02's host-native copy) and documentation
  reviews come from `qualification/manual.json`. FOC-02 needs three host
  terminals on the platform's OS; a host that fails is recorded as a
  limitation, not a failure (owner decision on #122).

Each record holds the source revision, the test selectors and the README
claiming them, the scenario's required layers, the archive's name, SHA-256
and provenance, the environment and configuration, the setup and expected
result from the matrix, what was observed, the commands with their results,
durations and logs, and any limitation. A failed case is a record, never a
failed job; only a harness error fails one.

The `verdict` job then judges every platform's records with the owner's
dispositions in `qualification/dispositions.json`. Unqualified conformance
needs every mandatory case passed on all three platforms; a case without a
platform's evidence is blocked. A case that did not pass needs the owner's
recorded disposition, which names its exact scenario, layer, states and
optionally platforms, so a performance (Measure) disposition never waives a
functional case. Without one the verdict fails and nothing is published (owner
decision on #122). Dispositions stay visible in `qualification.json` and the
job summary.

The evidence records are uploaded as `qualification-<platform>` and
`qualification-verdict` workflow artifacts and published as the release asset
`climon2-<version>-qualification.tar.gz` (owner decision on #122). Measure
reports exist only for release candidates (`-rc.N` tags): a final release
tag's Measure cases are not run unless the owner disposes of them.

Locally, on a native Linux host:

```sh
cargo xtask dist
cargo xtask qualify run --platform linux-x86_64 \
  --archive target/dist/climon2-0.1.0-linux-x86_64-gnu.tar.gz \
  --out target/qualification/linux-x86_64
cargo xtask qualify verdict --evidence target/qualification --out target/verdict
```

A local verdict lacks both macOS platforms, so their cases are blocked.

Publishing merges the independently generated checksum manifests, requires
identical stamped install scripts, verifies all assets, and attests and uploads
both archives with the qualification evidence. No native platform's failure
can be replaced by neutral checks.
