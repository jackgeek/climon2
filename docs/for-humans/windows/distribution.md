# Windows packaging and upgrades

This document says what a Windows release of climon2 contains, how it is
installed, and what an upgrade may and may not touch. It is for contributors.
All of it is built: the launcher's retry of [`current`](#current)
([#315](https://github.com/jackgeek/climon2/issues/315)),
[the archive](#the-archive) with its Dist job
([#319](https://github.com/jackgeek/climon2/issues/319)), and
[`install.exe`](#the-installer) and [`install.ps1`](#the-download-script)
with the Archive job that tests them
([#329](https://github.com/jackgeek/climon2/issues/329)), and
[publishing](#releases) the archive with a release
([#330](https://github.com/jackgeek/climon2/issues/330)). It was agreed in
[Define Windows packaging and upgrade contract](https://github.com/jackgeek/climon2/issues/215)
and is the specification the Windows port implements. The macOS and Linux
equivalent, which is built, is [Release distribution](../distribution.md).

It does not cover the monitor or the client library on Windows, which
[Define Windows monitor library and specification integration](https://github.com/jackgeek/climon2/issues/216)
decides, nor the CI jobs that build and test the archive, which
[Define Windows quality and CI qualification](https://github.com/jackgeek/climon2/issues/214)
decided.

## Terms

The [glossary](../../../GLOSSARY.md) defines the words used here. The ones that
matter most:

- The **launcher** is the small program on the user's `PATH`. It reads a file
  named `current` and starts the command-line program of the version that file
  names. Its behaviour is frozen
  ([ADR 0002](../../adr/0002-frozen-launcher-contract.md)).
- The **climon2 executable home** is the directory holding the launcher,
  `current` and one **version folder** per installed version. The **current
  version** is the one `current` names.
- A **release archive** is the single downloadable file for one version. The
  **installer** is the program inside it that places that version into an
  executable home and makes it current.
- A **session daemon** is the background process that runs a user's command.
  It keeps running across upgrades, using the files of the version that
  started it ([ADR 0001](../../adr/0001-version-matched-client-and-frozen-hello.md)).
- **ConPTY** is the Windows component that lets one program act as the
  terminal of another. climon2 ships its own copy
  ([ADR 0003](../../adr/0003-platform-and-shell-seams.md)).
- The **C runtime** is Microsoft's support library for compiled programs. Its
  `vcruntime140.dll` part is not included in Windows.
- **Mark-of-the-Web** is a tag Windows attaches to files that came from the
  internet. **SmartScreen** and **Smart App Control** are the Windows features
  that warn about, or block, unsigned programs.

## The command is `c2`

Windows ships one launcher, `c2.exe`. There is no `climon2.exe`, and no link
or copy under another name. Only the command is called `c2`: the product, its
other programs, its folders and its environment variables keep the name
climon2. The same change for Linux and macOS is
[Make c2 the only command name on Linux and macOS](https://github.com/jackgeek/climon2/issues/303).

## The archive

The archive is `climon2-<version>-windows-x86_64-msvc.zip`. As on the other
platforms, it holds one folder of the same name, which holds:

```
install.exe
c2.exe
LICENSE
README.md
<version>\climon2-cli.exe
<version>\climon2-daemon.exe
<version>\climon2-client.exe
<version>\climon2-monitor.exe
<version>\climon2_client.dll
<version>\conpty.dll
<version>\OpenConsole.exe
<version>\THIRD-PARTY-NOTICES.txt
```

- `climon2_client.dll` is the client library: the code that the
  [client](../../../GLOSSARY.md#sessions) program and the monitor load to talk to a
  session daemon.
- `install.ps1`, the download script, is stamped with the version and
  published beside the zip. The release's one `SHA256SUMS` lists both.
- Debug symbol files (`.pdb`) are not shipped.
- There are no test fixtures in the archive.

### How it is built

`cargo xtask dist` builds the archive, `install.ps1` and `SHA256SUMS` into
`target\dist` on an x86_64 Windows host. It writes the release's version into
the one `$Version = '…'` line of the checked-in
[`install.ps1`](../../../install.ps1), as the Unix dist stamps `install.sh`. The Windows Dist job runs it on `windows-2025` on
every run of the checks, even when the Windows test switch is off
([Windows quality](quality.md#the-switch)), and keeps the result as a
workflow artifact. The Release workflow publishes that zip and its stamped
`install.ps1` with the release
([#330](https://github.com/jackgeek/climon2/issues/330)).

- **Static C runtime.** Every climon2 program is built with the C runtime
  linked in, so a clean Windows 11 needs nothing installed first. The
  workspace's Cargo configuration (`.cargo/config.toml`) sets this for every
  Windows build, distribution builds included. Coverage builds replace that
  setting with their own flags, so xtask's coverage adds the same flag.
- **Import check.** The build fails if any shipped program or library imports
  `vcruntime140*.dll` or `msvcp140*.dll`, delay-loaded imports included. The
  CI runner has those installed, so without this check a missing-runtime
  fault would pass CI. The ConPTY files import the Universal C Runtime
  (`api-ms-win-crt-*`), which is part of Windows, and pass.
- **Format check.** Every program and library must be 64-bit x86 Windows code
  of the expected kind: the `.exe` files programs and the `.dll` files
  libraries. climon2's own files may not contain test-hook markers, as on the
  other platforms, and the client library must export only its bootstrap
  symbol.
- **Exact contents.** The build lists the finished zip and fails unless it
  holds exactly the files above, so a `.pdb` or any other stray file cannot
  ship. On Windows the client program and the client library both write
  debug symbols to `climon2_client.pdb`, and Cargo warns about the clash;
  the build copies each shipped file by its own name and checks its kind, so
  the clash cannot put a wrong file in the zip.
- **ConPTY files.** `conpty.dll` and `OpenConsole.exe` come from one pinned
  `Microsoft.Windows.Console.ConPTY` package. One checked-in file,
  [`pins/conpty.txt`](../../../pins/conpty.txt), records the package version, the
  package's SHA-256 and the SHA-256 of each of the two files. The build
  downloads the package, verifies all three and copies the two files into the
  version folder, using the same code as the test harness, which hosts its
  programs with the same files. A change to that file counts as a
  Windows-only change, so it runs the Windows quality jobs.
- **Notice.** `THIRD-PARTY-NOTICES.txt` carries the MIT notice of
  microsoft/terminal, beside the files it covers. Its source is
  [`pins/THIRD-PARTY-NOTICES.txt`](../../../pins/THIRD-PARTY-NOTICES.txt).

## Installing

### The download script

The documented command is:

```powershell
irm <release URL>/install.ps1 | iex
```

To pass options, run the script as a script block:

```powershell
& ([scriptblock]::Create((irm <release URL>/install.ps1))) -Dir <path> -NoModifyPath
```

- The script works under both Windows PowerShell 5.1, the only PowerShell
  Windows 11 includes, and PowerShell 7. CI runs it under both.
- It downloads the zip, verifies its SHA-256, expands it into a temporary
  folder, runs `install.exe` and removes the folder.
- It never calls `exit`, which under `iex` would close the user's window. A
  failure is a PowerShell error whose message starts
  `climon2-install.ps1: error:`, as the Unix script's do.
- Piping to `iex` works under the default execution policy, which blocks a
  saved `.ps1` file.
- Everything runs inside one script block, so `iex` leaves no variable or
  preference behind in the user's session.
- It hashes and expands the zip with .NET rather than `Get-FileHash` and
  `Expand-Archive`. Those come from script modules that Windows PowerShell
  started from PowerShell 7 cannot load, because it inherits PowerShell 7's
  module path.
- `CLIMON2_RELEASE_BASE_URL` replaces the release download URL, as it does for
  `install.sh`; the tests use it to serve a release locally.

A manual install verifies the zip against `SHA256SUMS`, expands it and runs
`install.exe`. Both routes behave the same from there, because everything
below is done by `install.exe`, not by the script.

### The installer

`install.exe [--dir <directory>] [--no-modify-path]` installs into
`%LOCALAPPDATA%\climon2` unless `--dir` names another executable home. Each
option may be given once, in either order. A relative `--dir` is taken from
the working directory, and the entry written to `PATH` is the absolute path.
As on the other platforms, an error prints one `climon2: error: …` line and
exits with 125. Session data stays in the climon2 home, which the installer
never touches.

### PATH

By default the installer adds the executable home to the user's `PATH`. This
needs no administrator rights.

- It reads the stored value of `HKCU\Environment\Path` without expanding
  variables, and writes it back as the same registry type. A user with no
  `Path` value of their own gets one of the type Windows uses for it,
  `REG_EXPAND_SZ`. A value that is not a string is left unchanged and the
  install fails. After a change it tells running programs that the
  environment changed, and prints one line saying it added the entry.
  Terminals that are already open do not see the change.
- The executable home is appended at the end, as a literal path, so `c2` never
  takes precedence over another tool.
- The installer records the exact entry it wrote, under `HKCU\Software\climon2`.
  A later install into a different directory removes the recorded entry and
  adds the new one, so there is never more than one climon2 entry.
- Entries are compared after expanding variables, ignoring letter case and a
  trailing backslash. An entry the user added by hand that already matches
  counts as present, and is not recorded as the installer's.
- `--no-modify-path` leaves `PATH` and the record untouched. It neither adds
  an entry nor removes an earlier one, and prints the entry to add by hand.
- The edit happens after the files are installed, while the installer still
  holds its lock, so two installers never edit `PATH` at once.

## Upgrading

Older versions are kept. A running session keeps using its own version's
files, so installing a new version never disturbs it.

### One installer at a time

The installer holds an exclusive lock on `install.lock` in the executable home
and fails at once if another installer holds it. The file is never renamed or
deleted. The Linux and macOS installers keep their lock on the directory
itself.

### Version folders

A new version is copied into a staging folder and renamed into place, so a
version folder is always complete.

Reinstalling a version that is already present:

| The files are | A program from that folder is running | Result |
| --- | --- | --- |
| Identical | Either | The folder is kept as it is |
| Different | No | The old folder is renamed aside, the new one is renamed in, and the old one is deleted |
| Different | Yes | The install is refused with 125 and nothing changes; the message says to end that version's sessions |

Windows cannot swap two folders in one step, so in the second row a launcher
started at that instant fails with its usual missing-version error.

Windows lets a folder be renamed while a program in it is running, so the
third row needs an explicit check: the installer tries to open each `.exe` in
the old folder for writing, which Windows refuses for a running program. Any
refusal means the folder is in use. Replacing it anyway would give a live
session daemon a client of a different build, which
[ADR 0001](../../adr/0001-version-matched-client-and-frozen-hello.md) forbids. A
session that starts between the check and the rename is not guarded against.

Only rebuilt development and release-candidate builds can meet the second and
third rows, because a published version never changes.

### The launcher

Every launcher can run every version, so replacing `c2.exe` is best effort:

- If the installed `c2.exe` is identical to the new one, it is left alone.
- Otherwise the installer renames the new launcher over it. This succeeds when
  no `c2` command is running.
- If Windows refuses, as it does while a `c2` command is running, the old
  launcher is kept. The install still succeeds, and prints one line saying the
  launcher was in use and was kept, with Windows' reason.

A changed launcher therefore arrives with the first install made while no `c2`
command is running. No running launcher is renamed aside, and the user is
never asked to stop a session.

### `current`

The last step writes the new `current` to a temporary file and renames it over
the old one. On Windows a reader can briefly find the file missing during that
rename, so the Windows launcher retries reading `current` for up to one second
before failing. This is part of the frozen launcher contract.

## Tests

The installer's own tests (`crates/climon2-install/tests/windows.rs`) run the
test program itself as `install.exe`, with `HKEY_CURRENT_USER` redirected to
a registry key of the test's own, so neither they nor the
[mutation tests](quality.md) can change the real user `PATH`. The
Windows Archive job ([Windows quality](quality.md#pull-request-jobs))
runs `crates/climon2-install/tests/archive_windows.rs` on the zip the Dist
job built: the archive's `install.exe`, and `install.ps1` under Windows
PowerShell 5.1 and PowerShell 7, served from a local web server, both as
`irm | iex` and as a script block with `-Dir` and `-NoModifyPath`. The
archive tests that change the real user `PATH` run only on that throwaway
runner, and put it back afterwards.

## Signing

The first Windows releases are not code-signed. As on Linux, their integrity
rests on `SHA256SUMS` and GitHub build-provenance attestations.

- Files installed with the download script carry no Mark-of-the-Web, and
  neither do files expanded with `Expand-Archive` or `tar`. A zip downloaded
  with a browser and extracted in Explorer does carry it, so SmartScreen may
  warn when `install.exe` runs. The [install guide](../guide/install.md) says to run `Unblock-File` on the
  zip first.
- Smart App Control, where it is enforcing, may block unsigned programs
  whatever their origin. The user guide lists this as a limitation.
- Neither feature exists on the CI runner. When the owner runs the
  [manual checklist](qualification.md#manual-checklist), they also
  record what SmartScreen and Smart App Control did for both install routes.
  This is an observation, not a pass-or-fail item.

Code signing is outside the Windows support effort. It becomes its own effort
if the observation shows the download-script route blocked on a default
Windows 11.

## Releases

One tag publishes Windows, Linux and macOS together, and a failure on any
platform blocks publication. The first Windows release has no predecessor, so
the previous-release compatibility check starts with the second.

## Measured facts

The file-system behaviour this document relies on was measured on the
`windows-2025` CI runner:
[Windows install probes](https://github.com/jackgeek/climon2/blob/a31b371d34db3036224caa5b2c2c7ac470b4836d/docs/research/windows-install-probes.md).
That runner has Defender's real-time scanning turned off, which may change
timings on a user's machine.
