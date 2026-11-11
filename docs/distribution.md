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
versions and tags, verifies every executable's architecture (and macOS
signatures), and rejects hook markers. Outputs go to `target/dist`:

- The native platform's versioned archive.
- `install.sh`, stamped with that exact version.
- `SHA256SUMS`, covering both files.

Each archive contains the installer `install`, launcher `climon2`, license,
README and `<version>/{climon2-cli,climon2-daemon,climon2-client}`. There are no
test fixtures or Cargo dependencies inside the archive.

## Installation

The published install script detects macOS or x86_64 GNU/Linux before
downloading. It verifies the selected archive with `shasum` on macOS or
`sha256sum` on Linux, extracts it, and runs its real installer. Quarantine
removal is macOS-only. Both platforms default to `$HOME/.local/share/climon2`;
`--dir` overrides the executable home without changing shell profiles.

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

Complete local qualification includes a mutation-load measurement before
mutation, even when no changed source needs mutation. After warming the
mutation build, five rounds each run 16 unchanged full-workspace test suites
concurrently, with a private warm-build copy per worker. No test is retried.
Every suite uses `--no-fail-fast`; any failing round stops qualification,
reports all failed suites' logs and retains every run's log under
`target/flake-logs/` in a unique directory. Worker builds are cleaned up.
Zero failures are required: a load-sensitive test can otherwise hide a
surviving mutant. This measurement runs locally, not in macOS native-only CI.

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

Before tagging, the maintainer must run `cargo xtask check --mutate-all` locally
for the exact release commit. CI cannot certify that local result. Tagged
releases require all macOS-exclusive source to pass CRAP and mutation in CI,
plus every native runtime and archive job.
Publishing merges the independently generated checksum manifests, requires
identical stamped install scripts, verifies all assets, and attests and uploads
both archives. No native platform's failure can be replaced by neutral checks.
