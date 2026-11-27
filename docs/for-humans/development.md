# Development and releases

How to build, test, check and release climon2.

Two diagrams show how the code fits together:

- [Executables and dynamic libraries](architecture/runtime-artifacts.md)
  shows the programs that run, the client library, the test tools, and how the
  monitor loads client libraries.
- The [workspace crate diagram](architecture/crate-relationships.md)
  shows which climon2 crates use which, including test tools.

```sh
cargo test --workspace --locked
cargo xtask setup-quality
cargo xtask check
cargo xtask check --print-mutation-plan
cargo xtask check --full
cargo xtask check --full --mutate-all
cargo xtask dist
cargo xtask install
cargo xtask install --dir "$HOME/tools/climon2"
```

## What you need

- The pinned Rust toolchain, a C compiler and libclang.
- ShellCheck, Python 3.11 or newer, GNU bash 5.x, vim and less.

Interactive tests use `CLIMON2_TEST_BASH` if set. Otherwise they look for
Homebrew bash on macOS, or `/bin/bash` and `/usr/bin/bash` on Linux. macOS
`/bin/bash` 3.2 is not used for these tests. If bash 5 is missing, CI fails;
locally the tests are skipped and the skip is reported. On Arch Linux, use
`CLIMON2_TEST_BASH=/usr/bin/bash`. `install.sh` still works with macOS
`/bin/bash` 3.2.

On arm64 macOS, add both Rust targets before you run workspace tests, `dist`
or `install`:
`rustup target add aarch64-apple-darwin x86_64-apple-darwin`.

## What CI runs

On Linux, CI runs the full runtime tests, real-terminal tests and tests of the
packaged build. It runs them on both macOS architectures too, but only while
the repository variable `CLIMON2_MACOS_TESTS` is `on`. Otherwise macOS only
builds its release archive. Windows runs only the tests that do not depend on
a platform.

Three agreed documents describe Windows support to come:

- [Windows host and shell qualification](windows/qualification.md)
  says what a Windows release will have to pass.
- [Windows quality and CI qualification](windows/quality.md)
  says how CI will check it.
- The [Windows platform and lifecycle contract](windows/lifecycle.md)
  says how its processes will start, run and end.

Two quality checks run on a dedicated Linux development machine, not in CI:

- **CRAP**, a score that combines a function's complexity with how much of it
  the tests cover.
- **Mutation testing**, which makes small changes (**mutants**) to the code
  and checks that some test fails for each one. A mutant no test catches is a
  **survivor**.

That machine runs them for shared and Linux-only code, with 8 mutation
workers.

The macOS quality job in CI also needs `CLIMON2_MACOS_TESTS` on. It runs
`cargo xtask mutate --native-only`. CRAP scores only the source files
registered as macOS-only, and mutation picks the changed functions in them.
Changes to shared code alone skip this job. Release tags use
`--native-only --mutate-all` to check every macOS-only file. Coverage still
runs the whole workspace, so real callers are exercised. Mutation still needs
the unchanged whole workspace to pass first in a worker. Native CI uses as
many CPUs as it has, up to 16.

## The quality gate

`xtask check` runs: formatting, strict clippy, unit, integration and
packaged tests (built by the installer), ShellCheck, append-only metadata
fixtures, proofs that instrumentation and worker isolation work, a CRAP score
of 5 or less on every shipped function (using coverage), and mutation testing
with no survivors.

It comes in two levels:

- The **ticket gate**, `cargo xtask check`, measures load once (see below) and
  mutates only the functions your change adds or edits.
  - A function, `const` or `static` counts as changed if it is new, or if the
    base has no item with the same path and the same tokens. A path looks like
    `<Pty as Drop>::drop` or `outer::inner`.
  - Whitespace, comments and doc comments do not count. So functions that only
    moved, or only had their docs edited, are not mutated.
  - Nested items are compared on their own, including those inside `const` and
    `static` values. An item owns every line its own code is on.
  - A changed `struct`, `enum`, `const`, `static` or `type` also selects every
    function in the same file whose code, or whose impl header, names it. The
    rule goes one step only
    ([#445](https://github.com/jackgeek/climon2/issues/445)).
  - Each import of a `use` is compared on its own. One that is added, removed
    or changed selects, in the same way, the functions that name it. A
    changed glob import, one imported as `_`, one that no function names (most
    likely a trait), or a glob of a changed item, marks the whole file.
  - A deleted or renamed item selects the functions that used it. A name in a
    format string (`"{LIMIT}"`) counts as naming it.
  - A file is mutated whole if a changed item has no such rule yet: for
    example an item macro, a trait method signature, an associated type or a
    `union`. So is a file whose base is not valid Rust.
  - Editing an impl or trait header (generics, bounds, where clause, self type,
    trait path or attributes), or the attributes of the module around it,
    changes every item inside.
  - Renamed files are compared with their old path. Added, copied and
    untracked files are mutated whole. Test-only code is skipped.
  - One mutation run covers every file with a chosen site, so the whole
    workspace baseline runs only once. Its `--lines` option applies to every
    file, so it gets all the lines that hold chosen sites in any file. These are
    found with the mutator's `--scan`, which runs no tests. A file may then
    also mutate a site on a line number chosen for another file. That is safe:
    it only ever mutates more, never less.
  - `cargo xtask check --print-mutation-plan` prints this choice and exits:
    each file, its chosen items, and its mutants, counting those it gains from
    another file's lines. The gate prints it too, before it mutates.
- The **full gate**, `cargo xtask check --full`, measures load five times and
  mutates whole changed files. Run it once before a feature branch merges into
  `main`, with `CLIMON2_MUTATION_BASE=origin/main`.

`--mutate-all` mutates every shipped file whole, at either level.
`cargo xtask mutate`, which the [quality runners](quality-runners.md) run,
chooses as the ticket gate does, and does not measure load.

**Build check.** Like `cargo xtask test`, the gate's tests first run the
monitor's build test alone. This proves the plain monitor binary has no test
hooks. Only then do the workspace tests build it with hooks at the same path.

**How a mutant is tested.** The unchanged workspace must pass inside a real
mutation copy before mutation starts. A worker that breaks for its own reasons
can never count as a caught mutant. Each mutant runs in stages:

1. the workspace library tests;
2. its own crate's integration tests;
3. the whole workspace.

A failing stage stops the run at once. A mutant that passes the first two
stages still runs the whole workspace before it can count as a survivor. The
unchanged baseline always runs the whole workspace.

**Stage time limits.** After coverage, with nothing else running, the gate
times each of the first two stages for every target once, in the warm
`target/mutation-tests` build. It writes the **stage time limits** (10 times
each run, at least 2 s) to `target/mutation-stage-timeouts.json`. A mutant
builds each of those stages, then runs its tests under that stage's limit. If
a stage runs past it, the whole process group is killed and the mutant counts
as caught, the same as with the mutator's own time limit. So a test that hangs
a mutant's library stage frees its worker in seconds, not after 10 whole
workspace runs. The whole-workspace stage has no stage limit. The mutator's
limit for the whole run (10 times the unchanged whole-workspace run, at least
2 s) still applies.

**Load measurement.** Both levels measure how tests behave under load before
mutating, even when no changed shipped code needs mutating. The gate first
times one whole-workspace run with nothing else running, in the warm build.
Then it runs its rounds (one for the ticket gate, five for the full gate). Each
round runs 8 suites at once, each in its own copy of the warm build
(copy-on-write where the file system supports it). Each suite acts out a
mutant's run on the unchanged workspace, in its own session: the first two
stages in order, each under its stage limit, then the whole workspace, all
inside the mutator's limit.

A suite fails if any stage has a failing test or runs past its limit. Either
result would count a mutant as caught and hide a survivor. A failure under
load is a bug in the tests, so nothing is retried. The measurement stops after
the failing round. It names each failing test and test target, how many suites
each failed in, and the limit each killed suite hit. Then it lists their logs.
Logs from every run stay in their own folder under `target/flake-logs/`.
Worker builds are deleted afterwards. The next measurement deletes any left by
one that was killed.

Workers get their own copies of the warm build (copy-on-write where
supported). They do not rebuild dependencies from scratch for each changed
file.

**Choosing the base.** `CLIMON2_MUTATION_BASE` sets what the changes are
compared with (default `HEAD`). Both levels compare with the merge base of
that revision and `HEAD`. So a base like `origin/main` that has moved on does
not pull in its own changes. For committed work, set it to the target branch
or the merge base, for example
`CLIMON2_MUTATION_BASE=origin/main cargo xtask check --full`.

Code for one platform must live in source files registered for that platform.
Shared files must not hide macOS-only code that Linux cannot check.

## Building release archives

`xtask dist` needs either an **arm64 macOS host** with both Rust macOS
targets, or an **x86_64 GNU/Linux host**.

- macOS builds keep deployment target 15.0, universal binaries and signature
  checks.
- Linux builds check that the programs are native ELF files. Published Linux
  builds are made on Ubuntu 24.04, not Arch, to set the lowest glibc version
  they need.

Both builds keep the packages without test hooks apart. They write the
platform's archive, a stamped `install.sh` and `SHA256SUMS` to `target/dist/`.
Each version folder also holds the client library: `libclimon2_client.dylib`
on macOS or `libclimon2_client.so` on Linux. It must export only its start-up
function and report exactly the archive's version. The Linux library must also
have no soname, RPATH or RUNPATH, and need only glibc's libraries.
`release-check` runs these library checks on the local per-package release
build too. Set `CLIMON2_E2E_ARCHIVE` to the archive to test it as it is.

On an **x86_64 Windows host**, `xtask dist` builds the Windows zip, the
download script `install.ps1`, and their `SHA256SUMS` instead, with the pinned
ConPTY files. It fails if any shipped program or library needs the Visual C++
runtime. The zip's `install.exe` installs it, and each release publishes the
zip and the stamped `install.ps1`. See
[Windows packaging and upgrades](windows/distribution.md).

`xtask install` builds and checks the same release files as `dist`, then runs
the installer without making an archive. It works the same on every `dist`
host, and needs the same host and tools as `dist`. By default it installs to
`~/.local/share/climon2` on macOS and Linux and to `%LOCALAPPDATA%\climon2` on
Windows; `--dir <directory>` picks another folder. It keeps other versions.

It never puts the folder on your `PATH`: it does not touch shell start-up
files, and on Windows it runs `install.exe` with `--no-modify-path`, so your
user `PATH` stays as it was. The installer prints the entry to add instead. To
install on Windows as a release would, with the `PATH` change, run
`cargo xtask dist` and the `install.exe` in the zip it makes.

## Releasing

Before a release, the maintainer must run
`cargo xtask check --full --mutate-all` locally on the exact release commit and
see it pass. CI does not repeat it.

A `v<workspace version>` tag runs:

1. full macOS-only mutation, and the native runtime and archive checks (on
   macOS only while `CLIMON2_MACOS_TESTS` is on);
2. [release qualification](distribution.md#release-qualification)
   of the exact archives on all four platforms, whatever that variable says.

The same final macOS archive must pass on both architectures. The Linux
archive must pass on Ubuntu 24.04, under bash 5, with vim and less. The
Windows zip must pass on `windows-2025`, with the Windows Test and Archive
jobs and the native-only gate over every Windows-only file
([Windows quality](windows/quality.md#release)). Every platform must pass
before one GitHub Release is published, with the Windows zip, `install.ps1`
and a provenance attestation for each file.

`-rc.N` tags are prereleases, published without the Windows manual checklist.
A final tag needs the checklist's records of its latest release candidate. From the second published release on, it must
work with the release before it. The first release has nothing to be
compatible with.

**No Intel runner, no release.** If GitHub retires the `macos-15-intel`
runner, a new native Intel macOS 15+ runner must pass the same archive tests.
A missing runner, or passing on arm64 alone, does not prove Intel support.
