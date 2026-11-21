# Windows quality and CI qualification (agreed, not implemented)

This document says how Windows code is checked: which CI jobs run, how
Windows-only source meets the same quality thresholds as the rest of climon2,
and what a release needs from Windows before it is published. It is for
contributors. Nothing here is built yet; it was agreed in
[Define Windows quality and CI qualification](https://github.com/jackgeek/climon2/issues/214)
and is the specification the Windows port implements.

[Windows host and shell qualification](windows-qualification.md) says which
scenarios run and what the manual checklist contains. This document says where
they run and how their results are stored.

## Terms

The [glossary](../GLOSSARY.md) defines the words used here. The ones that
matter most:

- **Shared source** is compiled on every platform. **Windows-only source** is
  compiled only on Windows.
- A **registered native file** is a source file listed in xtask's native-source
  registry as belonging to one operating system. Today the registry lists
  `linux.rs` and `macos.rs` files.
- **CRAP** is a score combining a function's complexity with how little of it
  tests cover; climon2 requires 5 or less. **Mutation testing** changes the
  code in small ways (each change is a **mutant**) and reruns the tests: a
  mutant the tests catch is **killed**, one they miss is a **survivor**.
  climon2 requires zero survivors.
- The **native-only gate** is `cargo xtask mutate --native-only`: CRAP and
  mutation over the registered native files of the machine it runs on. macOS
  CI runs it today.
- A [release gate](../GLOSSARY.md) is a check that must pass before a version
  is released. [Release qualification](../GLOSSARY.md) is the release gate
  that runs every scenario on the exact archives about to be published and
  stores an [evidence record](../GLOSSARY.md) for each. A
  [disposition](../GLOSSARY.md) is the owner's recorded decision to accept a
  case that did not pass.
- A **release candidate** is a tag ending `-rc.N`, published as a prerelease.

## Where Windows runs

CI is the only environment that qualifies Windows. There is no Windows
development machine. The owner's Windows 11 virtual machine is used for the
manual checklist and for measuring facts, and no pull request check depends on
it.

All Windows jobs run on the GitHub runner `windows-2025`, named exactly, not
`windows-latest`. Its image changes weekly and cannot be pinned, so every
Windows evidence record carries the runner image version and the Windows build
number.

## Pull request jobs

| Job | What it does |
| --- | --- |
| Test | Every workspace test, including the harness's host-model scenarios |
| Quality | The native-only gate over registered Windows files |
| Dist | Builds the release zip |
| Archive | Runs the end-to-end tests and an `install.ps1` smoke test on that exact zip |
| Neutral (existing) | Its Windows leg stays: `climon2-core`'s tests on Windows |

### The switch

A repository variable, `CLIMON2_WINDOWS_TESTS`, works like
`CLIMON2_MACOS_TESTS`. It defaults to on. When it is off:

- Test, Quality and Archive are skipped. Dist still builds the zip, and
  Neutral still proves shared source compiles and `climon2-core` passes on
  Windows.
- Release qualification runs every Windows job regardless.
- A pull request that touches Windows-only code runs Test and Quality
  regardless. Without this, such a change would merge having run nowhere.

A pull request touches Windows-only code when it changes any of:

- a registered Windows file;
- the Windows host in `climon2-test-term`;
- the Windows scenario tests;
- the pinned hashes of the ConPTY package, vim or Microsoft Edit;
- the workflow files or xtask's quality code.

## Source ownership

Windows-only code lives only in registered `windows.rs` files, listed
explicitly as the Linux and macOS files are. Outside them, `cfg(windows)` may
appear only on a `mod` declaration. A source check that runs on Linux, as part
of the [ticket gate](../GLOSSARY.md), fails on any other use, because such
code would be compiled only on Windows and mutated nowhere.

| Source | Tests must pass on | CRAP and mutation run on |
| --- | --- | --- |
| Shared | Linux, macOS, Windows | the Linux development host |
| Registered Windows files | Windows | Windows CI, by the native-only gate |
| The harness's Windows host (`climon2-test-term`) | Windows | nowhere: it is test code, as on every platform |

A test of shared source that fails only on Windows blocks the pull request.
Ownership of that source does not move.

## The native-only gate on Windows

The gate needs three changes before its result means anything on Windows.

**The mutator.** The pinned mutator has two faults on Windows: it runs its test
command through `cmd.exe`, where the Python path variable is not expanded, and
it crashes on the first timeout. Both are fixed in the mutator itself, on a
fork the owner controls, pinned by revision and offered upstream. xtask also
passes the test command with the Python path already expanded, on every
platform, so no shell expands a variable.

**The sentinel.** If every mutant failed to start, each would be scored as
killed and the gate would pass having tested nothing. So before the gate,
`cargo xtask mutation-probe` plants two mutants in a fixture: one the tests
must kill and one that must survive. The gate fails unless both come out
exactly so.

**Coverage.** `coverage.rs` gains a Windows arm that passes
`-runtime-counter-relocation`. The gate starts with a probe showing that a
process ended by `TerminateProcess` keeps its coverage counters, and fails if
it does not; the daemon is often ended that way in tests.

**Cost.** When no registered Windows file changed, the gate exits before doing
any mutation work. A release mutates every registered Windows file. That run
is measured in the first implementation slice against a budget of 90 minutes;
over budget, the files are split across parallel jobs.

## Facts only CI can show

These decisions in [ADR 0003](adr/0003-platform-and-shell-seams.md) rest on
Windows behaviour. Each is an ordinary test in the Test job:

- the error codes for connecting to a missing socket path and a stale one, and
  for deleting a socket whose listener is alive;
- the runtime directory's access control denying a second, non-administrator
  account, which the job creates;
- the Authenticode signatures of the bundled `conpty.dll` and
  `OpenConsole.exe`;
- binding the runtime directory on the runner's real `%LOCALAPPDATA%` volume.

The memory used by the daemon and console host, under the inbox and the
bundled ConPTY, is written to the job summary on each run. It has no
threshold.

vim and Microsoft Edit are fetched from their upstream release URLs, verified
against the SHA-256 in the repository and cached by that hash. A failed
download or a mismatch fails the job. No mirror is kept.

## Release

`windows-x86_64` becomes the fourth release qualification platform. A release
needs all of these from Windows, whatever the switch says:

1. Test and Archive pass.
2. The native-only gate passes over every registered Windows file, after the
   sentinel and coverage probes.
3. Every automated scenario case has passed or has a disposition.
4. Every manual checklist case has passed or has a disposition. This applies
   to final tags only.

### Evidence records

`cargo xtask qualify run` records the existing acceptance matrix on Windows as
it does elsewhere. It also reads the scenario IDs in
[Windows host and shell qualification](windows-qualification.md):

| IDs | Scenarios | Layer |
| --- | --- | --- |
| `WIN-01` to `WIN-11` | The Windows-only scenario groups | Contract |
| `WINM-01` to `WINM-16` | The manual checklist items | Host manual |

A manual checklist case passes when `qualification/manual.json` holds a passed
record for both Windows Terminal and VS Code (`WINM-12` needs VS Code only). A
failed or missing record is a failed or not-run case that needs a disposition.
This is stricter than the existing Host manual check, `FOC-02`, where a host
that fails is recorded as a limitation.

### Manual checklist timing

The checklist needs the candidate installed with `install.ps1`, so it can only
run once a release candidate is published:

1. A release candidate tag publishes its prerelease. Its manual checklist
   cases are recorded as not run and do not block it.
2. The owner runs the checklist on that candidate and commits records naming
   its version and the SHA-256 of its Windows zip.
3. The final tag publishes only if those records name the latest release
   candidate, and the final tag's source differs from that candidate's only in
   the version stamp and the files under `qualification/`.
