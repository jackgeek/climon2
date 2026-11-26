# Windows quality and CI qualification

This document says how Windows code is checked: which CI jobs run, how
Windows-only source meets the same quality thresholds as the rest of climon2,
and what a release needs from Windows before it is published. It is for
contributors. The Test job (see [Pull request jobs](#pull-request-jobs)), its switch and change trigger, the
registry's Windows arm and the source check are built
([#313](https://github.com/jackgeek/climon2/issues/313)), and so is the
Quality job with everything [the native-only gate on Windows](#the-native-only-gate-on-windows)
needs ([#314](https://github.com/jackgeek/climon2/issues/314)), and the
harness's Windows host, whose own tests the Test job runs
([#320](https://github.com/jackgeek/climon2/issues/320)), and the Dist job
([#319](https://github.com/jackgeek/climon2/issues/319)), and the Archive job
([#329](https://github.com/jackgeek/climon2/issues/329)), which so far tests
the installer and the download script on the zip. The Test job runs the
tests that build on Windows, the Windows forms of the session suites among
them ([#328](https://github.com/jackgeek/climon2/issues/328)); the Quality
job runs those listed as `WINDOWS_SUITE` in `xtask/src/quality.rs`. The
session suites in `climon2-e2e` and the monitor acceptance matrix's own
suites do not build there yet, and the Archive job
runs no end-to-end suite yet. What a [release](#release) needs from Windows is
built ([#330](https://github.com/jackgeek/climon2/issues/330)), except
measuring PERF-01 on Windows. Also not built: the Authenticode test and the memory summary under
[Facts only CI can show](#facts-only-ci-can-show). It was agreed in
[Define Windows quality and CI qualification](https://github.com/jackgeek/climon2/issues/214)
and is the specification the Windows port implements.

[Windows host and shell qualification](qualification.md) says which
scenarios run and what the manual checklist contains. This document says where
they run and how their results are stored.

## Terms

The [glossary](../../../GLOSSARY.md) defines the words used here. The ones that
matter most:

- **Shared source** is compiled on every platform. **Windows-only source** is
  compiled only on Windows.
- A **registered native file** is a source file listed in xtask's native-source
  registry as belonging to one operating system. Today the registry lists
  `linux.rs` and `macos.rs` files, and twenty-two `windows.rs` files:
  `crates/climon2-platform/src/home/windows.rs`, which names the Windows
  homes; `crates/climon2-platform/src/private/windows.rs`, which creates the
  files and folders only their user may open
  ([#339](https://github.com/jackgeek/climon2/issues/339));
  `crates/climon2-platform/src/runtime/windows.rs`,
  `crates/climon2-platform/src/waiting/windows.rs`,
  `crates/climon2-platform/src/process/windows.rs` and
  `crates/climon2-platform/src/presentation/windows.rs`, the runtime
  directory, sockets, waiting, daemon identity and `list`'s presentation
  ([#316](https://github.com/jackgeek/climon2/issues/316));
  `crates/climon2-platform/src/daemon/windows.rs`, which starts the session
  daemon; `crates/climon2-platform/src/pty/windows.rs`, which hosts the
  session's command in the bundled ConPTY and its session job;
  `crates/climon2-platform/src/peer/windows.rs`,
  `crates/climon2-platform/src/terminal/windows.rs` and
  `crates/climon2-platform/src/logging/windows.rs`, the daemon's
  connections, signals and captured stderr; and
  `crates/climon2-daemon/src/pty/windows.rs`, the daemon's side of its
  pseudoconsole ([#317](https://github.com/jackgeek/climon2/issues/317));
  `crates/climon2-platform/src/library/windows.rs`,
  `crates/climon2-platform/src/notifier/windows.rs`,
  `crates/climon2-platform/src/connection/windows.rs` and
  `crates/climon2-platform/src/native_path/windows.rs`, the client library's
  loading, notification and waker, sends and native paths
  ([#318](https://github.com/jackgeek/climon2/issues/318));
  `crates/climon2-platform/src/command/windows.rs`, which resolves the
  session's command and builds its command line
  ([#322](https://github.com/jackgeek/climon2/issues/322));
  `crates/climon2-platform/src/handoff/windows.rs`, the CLI's spawn-and-wait
  hand-off to the client ([#321](https://github.com/jackgeek/climon2/issues/321),
  which also gave `terminal/windows.rs` the client's console and
  `waiting/windows.rs` its waits on events);
  `crates/climon2-launcher/src/handoff/windows.rs`, the launcher's
  hand-off; `crates/climon2-install/src/windows.rs` and
  `crates/climon2-install/src/user_path/windows.rs`, the installer and its
  user PATH edit ([#329](https://github.com/jackgeek/climon2/issues/329)); and
  `crates/climon2-cli/src/windows.rs`, which runs `list`, `exec`, `attach`
  and `kill` and hands `monitor` to the monitor program
  ([#327](https://github.com/jackgeek/climon2/issues/327)).
- **CRAP** is a score combining a function's complexity with how little of it
  tests cover; climon2 requires 5 or less. **Mutation testing** changes the
  code in small ways (each change is a **mutant**) and reruns the tests: a
  mutant the tests catch is **killed**, one they miss is a **survivor**.
  climon2 requires zero survivors.
- The **native-only gate** is `cargo xtask mutate --native-only`: CRAP and
  mutation over the registered native files of the machine it runs on. macOS
  CI runs it today.
- A [release gate](../../../GLOSSARY.md) is a check that must pass before a version
  is released. [Release qualification](../../../GLOSSARY.md) is the release gate
  that runs every scenario on the exact archives about to be published and
  stores an [evidence record](../../../GLOSSARY.md) for each. A
  [disposition](../../../GLOSSARY.md) is the owner's recorded decision to accept a
  case that did not pass.
- A **release candidate** is a tag ending `-rc.N`, published as a prerelease.

## Where Windows runs

CI is the only environment that qualifies Windows. The owner's Windows 11
virtual machine is used for the manual checklist, for measuring facts, and for
development sessions on tickets about Windows run-time behaviour. It never
qualifies a change: the pull request's Windows CI run is the proof, and no
pull request check depends on the virtual machine.

All Windows jobs run on the GitHub runner `windows-2025`, named exactly, not
`windows-latest`. Its image changes weekly and cannot be pinned, so every
Windows evidence record carries the runner image version and the Windows build
number.

## Pull request jobs

| Job | What it does |
| --- | --- |
| Test | Every workspace test, including the harness's host-model scenarios. It waits for Dist and runs the smoke test against its zip ([#321](https://github.com/jackgeek/climon2/issues/321)) |
| Quality | The native-only gate over registered Windows files; during development, only when its own switch is on (see [The Quality job during development](#the-quality-job-during-development)) |
| Dist | Builds the release zip with `cargo xtask dist`, after testing that command's own checks and `cargo xtask install` |
| Archive | Runs the end-to-end tests and an `install.ps1` smoke test on that exact zip, under Windows PowerShell 5.1 and PowerShell 7 |
| Neutral (existing) | Its Windows leg stays: `climon2-core`'s tests on Windows |

### The switch

A repository variable, `CLIMON2_WINDOWS_TESTS`, works like
`CLIMON2_MACOS_TESTS`. It defaults to on. When it is off:

- Test, Quality and Archive are skipped. Dist still builds the zip, and
  Neutral still proves shared source compiles and `climon2-core` passes on
  Windows.
- Release qualification runs every Windows job regardless.
- A pull request that touches Windows-only code runs Test, Quality and
  Archive regardless. Without this, such a change would merge having run nowhere.

A pull request touches Windows-only code when it changes any of:

- a registered Windows file;
- the Windows host in `climon2-test-term` (`src/windows.rs`, `src/windows/`)
  and its tests (`tests/windows_host.rs`);
- the Windows scenario tests, and the installer's Windows tests and archive
  tests (`crates/climon2-install/tests/windows.rs` and
  `archive_windows.rs`);
- the Windows dist (`xtask/src/windows_dist.rs`) and the download script
  (`install.ps1`), which make the zip the Archive job tests;
- the pinned downloads under `pins/`: the ConPTY package's
  `pins/conpty.txt`, and vim's and Microsoft Edit's, `pins/vim.txt` and
  `pins/edit.txt`;
- the workflow files or xtask's quality code.

### The Quality job during development

The Quality job is off for pull requests and pushes to `main`. It was turned
off while Windows support was built. A run takes 25 to 50 minutes, and each round
of fixes waited for one. The gate runs on the Windows development VM instead
(see [Running the gate locally](#running-the-gate-locally)), and each pull
request that changes a registered Windows file reports its result. This was
the owner's decision on 2026-11-22.

A second repository variable, `CLIMON2_WINDOWS_QUALITY`, turns the job back
on when it is `on`. It stays off: the owner's Windows machine runs the same
gate as a self-hosted runner on every push, and that is the check for each
change (owner decision on
[#330](https://github.com/jackgeek/climon2/issues/330); see
[Quality runners](../quality-runners.md)). Release qualification and tags run
the job on `windows-2025` whatever the variable says.

### Running the gate locally

The VM needs Python 3.11 or later and
[cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov); `setup-quality`
installs the pinned tools and builds `cargo-llvm-cov` when it is missing. From
the worktree of the branch, in Git Bash:

```sh
export CLIMON2_QUALITY_PYTHON="$LOCALAPPDATA/Programs/Python/Python313/python.exe"
export CLIMON2_MUTATION_BASE=origin/main
export CLIMON2_NATIVE_MUTATION_WORKERS=2
cargo xtask setup-quality
cargo xtask mutate --native-only
```

- `CLIMON2_MUTATION_BASE` makes the gate mutate the registered Windows files
  changed since `main`, as the job does on a pull request.
- `CLIMON2_NATIVE_MUTATION_WORKERS` caps the workers. Without it the gate uses
  one per CPU, up to 16, and 16 workers' builds do not fit in the VM's 16 GB.
  Use 2 when another build or gate is running, and at most 4 otherwise.
- The gate runs the sentinel and the coverage probe first, as on CI. It fails
  if any function in a changed file scores over 5 or any mutant survives.

The cross-account test is skipped, saying why, unless
`CLIMON2_SECOND_ACCOUNT` and `CLIMON2_SECOND_ACCOUNT_PASSWORD` name a second,
non-administrator account. The Test job still runs it on every pull request.

The VM runs with UAC off, so the gate runs with a full administrator token,
as the runner does.

## Source ownership

Windows-only code lives only in registered `windows.rs` files, listed
explicitly as the Linux and macOS files are. Outside them, `cfg(windows)` may
appear only on a `mod` declaration. A source check that runs on Linux, as part
of the [ticket gate](../../../GLOSSARY.md), fails on any other use, because such
code would be compiled only on Windows and mutated nowhere.

| Source | Tests must pass on | CRAP and mutation run on |
| --- | --- | --- |
| Shared | Linux, macOS, Windows | the Linux development host |
| Registered Windows files | Windows | Windows CI, by the native-only gate |
| The harness's Windows host (`climon2-test-term`) | Windows | nowhere: it is test code, as on every platform |

A test of shared source that fails only on Windows blocks the pull request.
Ownership of that source does not move.

## The native-only gate on Windows

The gate needs three changes before its result means anything on Windows. It
also tests only what builds on Windows: the tests the Test job runs, less the
end-to-end scenarios that are not the only test covering a registered Windows
file (owner decision on
[#328](https://github.com/jackgeek/climon2/issues/328), 2026-11-22). The
list is `WINDOWS_SUITE` in `xtask/src/quality.rs`.

**The mutator.** The upstream mutator has two faults on Windows: it runs its
test command through `cmd.exe`, where the Python path variable is not
expanded, and it crashes on the first timeout. Building
[#314](https://github.com/jackgeek/climon2/issues/314) found two more. It
cannot delete a mutant's working copy once the build inside it passes
Windows's 260-character path limit. And it writes each mutant in text mode,
which turns the CRLF line endings of a Windows checkout into CR CR LF, so
every Rust mutant fails to build and counts as killed; the sentinel below
caught this on the first CI run. All four are fixed in the mutator itself,
on the owner's fork [jackgeek/mutator](https://github.com/jackgeek/mutator)
(branch `windows-runner`), pinned by revision and to be offered upstream.
xtask also passes the test command with the Python path already expanded, on
every platform, so no shell expands a variable.

**The sentinel.** If every mutant failed to start, each would be scored as
killed and the gate would pass having tested nothing. So the Windows gate
first plants two mutants in a fixture, the same check `cargo xtask
mutation-probe` runs: one the tests must kill and one that must survive. The
gate fails unless both come out exactly so.

**Coverage.** `coverage.rs` gains a Windows arm that passes
`-runtime-counter-relocation`. The gate starts with a probe showing that a
process ended by `TerminateProcess` keeps its coverage counters, and fails if
it does not; the daemon is often ended that way in tests.

**Timeouts.** On Windows a mutant's tests are the `WINDOWS_SUITE` runs, in
order. Before any mutant runs, the gate times one unloaded run of them, and
each mutant's whole run then has 10 times that. A mutant that outlives it
counts as killed. When each run ends, whether it passed, failed or timed out,
`tools/test-mutation.py` ends every process the tests started, so a test that
starts a `climon2-daemon` and hangs leaves nothing on the machine. It finds
those processes in the process list: by parent, checked against creation
times because Windows reuses the ID of a process that has exited, and by an
executable in the worker's build folder or in the private temporary folder
(`TMP` and `TEMP`) it gives the tests. The second finds a daemon whose parent
has exited. The Quality job on the owner's VM fails after 120 minutes; its
longest passing run took 68
([#401](https://github.com/jackgeek/climon2/issues/401); see
[Quality runners](../quality-runners.md)).

**Runtime folders.** Tests that start sessions set `LOCALAPPDATA` to a folder
of their own in `%LOCALAPPDATA%\c2t`, not in the private temporary folder,
which is too long for a 107-byte socket path. The folder may be at most 51
bytes; where `%LOCALAPPDATA%` is too long for that, set
`CLIMON2_TEST_SHORT_ROOT` to a shorter folder to use instead of `c2t`. Each
folder holds a lock file open while its test runs. Each new folder first
removes any over an hour old whose lock no one holds, which a killed run left
behind ([#422](https://github.com/jackgeek/climon2/issues/422)).

**Cost.** When no registered Windows file changed, the gate exits before doing
any mutation work. A release mutates every registered Windows file. That run
is measured in the first implementation slice against a budget of 90 minutes;
over budget, the files are split across parallel jobs. Its first measurement,
on [#343](https://github.com/jackgeek/climon2/pull/343) with one registered
Windows file and no mutation sites in it, took 8.5 minutes on `windows-2025`
(11 minutes for the whole job). Every run of the Quality job writes its time
to the job summary, and a release run compares it with the budget there.

## Facts only CI can show

These decisions in [ADR 0003](../../adr/0003-platform-and-shell-seams.md) rest on
Windows behaviour. Each is an ordinary test in the Test job:

- the error codes for connecting to a missing socket path and a stale one, and
  for deleting a socket whose listener is alive;
- the runtime directory's access control denying a second, non-administrator
  account, which the job creates; the same account is also denied opening a
  session log, a metadata file and the `logs` folder;
- the Authenticode signatures of the bundled `conpty.dll` and
  `OpenConsole.exe`, which no test checks yet;
- binding the runtime directory on the runner's real `%LOCALAPPDATA%` volume.

[#316](https://github.com/jackgeek/climon2/issues/316) built the first,
second and fourth as tests in `climon2-platform`, listed in its README. On a
Windows 11 VM a missing socket path and a stale socket both refuse with
`WSAECONNREFUSED`, a missing runtime directory fails with `WSAENETDOWN`, and a
live socket file can be deleted. Both Windows jobs create the second account,
a local member of Users with a random password, and name it to the tests in
`CLIMON2_SECOND_ACCOUNT` and `CLIMON2_SECOND_ACCOUNT_PASSWORD`. The test that
uses it is skipped, saying why, where the account is missing, and fails on CI.

The memory used by the daemon and console host, under the inbox and the
bundled ConPTY, is to be written to the job summary on each run, with no
threshold. No job writes it yet.

vim and Microsoft Edit are fetched from their upstream release URLs,
verified against the SHA-256s in `pins/vim.txt` and `pins/edit.txt` and cached
by the zip's hash, in `target/apps` where the Test job's cache keeps them
([#328](https://github.com/jackgeek/climon2/issues/328)). A failed download
or a mismatch fails the job. No mirror is kept.

## Release

`windows-x86_64` is the fourth release qualification platform
([#330](https://github.com/jackgeek/climon2/issues/330)). A release needs all
of these from Windows, whatever the switches say:

1. Test and Archive pass. The Release workflow calls the checks for its tag,
   which always run the Windows jobs.
2. The native-only gate passes over every registered Windows file, after the
   sentinel and coverage probes: Release calls the checks with `mutate_all`,
   which runs the Quality job's `cargo xtask mutate --native-only
   --mutate-all` on `windows-2025` whatever `CLIMON2_WINDOWS_QUALITY` says.
   The owner's own machines ([Quality runners](../quality-runners.md)) do
   not take part.
3. Every automated scenario case has passed or has a disposition.
4. Every manual checklist case has passed or has a disposition. This applies
   to final tags only.

### Evidence records

The Release workflow's `qualify-windows` job runs `cargo xtask qualify run
--platform windows-x86_64` on the exact zip on `windows-2025`. It records the
existing acceptance matrix on Windows as it does elsewhere, from two runs: the
Windows suite the Quality job tests, on hook-enabled debug builds, and the
CLI's tests and the installer's archive tests against the zip. It also reads
the scenario tables in [Windows host and shell qualification](qualification.md),
which apply to Windows alone:

| IDs | Scenarios | Layer |
| --- | --- | --- |
| `WIN-01` to `WIN-11` | The Windows-only scenario groups | Contract |
| `WINM-01` to `WINM-16` | The manual checklist items | Host manual |

`WIN-01` to `WIN-11` take their results from the tests that
`crates/climon2-cli/README.md` names for them. Every Windows record also
carries the runner image version (`runner_image`, such as `win25 20251110.1`)
and the Windows build (`windows_build`, from `ver`) in its `environment`. The
verdict blocks a Windows record that lacks either.

A manual checklist case passes when `qualification/manual.json` holds a passed
record for each host its row names: Windows Terminal and VS Code, or VS Code
alone for `WINM-12`. A failed or limited record is a failed case, and a
missing one a not-run case; either needs a disposition. This is stricter than
the existing Host manual check, `FOC-02`, where a host that fails is recorded
as a limitation. A checklist record names the candidate it was run on, by its
version and its zip's SHA-256, and only records of the latest candidate count:

```json
{
  "scenario": "WINM-02", "layer": "Host manual", "os": "windows",
  "host": "Windows Terminal", "gesture": "c2 exec pwsh, a command, detach, c2 list, attach",
  "observed": "the screen was restored", "result": "passed",
  "recorded": "owner, 2026-11-24",
  "candidate": "0.2.0-rc.1", "sha256": "<the zip's SHA-256 from the release's SHA256SUMS>"
}
```

### Manual checklist timing

The checklist needs the candidate installed with `install.ps1`, so it can only
run once a release candidate is published:

1. A release candidate tag publishes its prerelease. Its manual checklist
   cases are recorded as not run and do not block it: the verdict, given the
   tag, gives each one the release rule's disposition, which stays visible in
   the records.
2. The owner runs the checklist on that candidate and commits records naming
   its version and the SHA-256 of its Windows zip.
3. The final tag publishes only if those records name the latest release
   candidate, and the final tag's source differs from that candidate's only in
   the version stamp and the files under `qualification/`. The Release
   workflow's `candidate` job runs `cargo xtask qualify candidate`, which finds
   the published `-rc.N` of the tag's version with the greatest `N` and
   compares every changed file after replacing the candidate's version with
   the final one. It fails when there is no candidate or anything else
   changed, and nothing is published. Otherwise it passes the candidate's
   version and zip SHA-256 to the Windows qualification job.

A release publishes every platform or none. Publishing needs the checks, the
candidate job and the verdict to pass, and uploads the Windows zip and its
stamped `install.ps1` beside the Unix archives, with one `SHA256SUMS` over all
of them and a build-provenance attestation for each file.
