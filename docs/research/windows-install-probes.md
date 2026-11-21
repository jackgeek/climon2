# Windows install probes: results

Measurements for
[Define Windows packaging and upgrade contract (#215)](https://github.com/jackgeek/climon2/issues/215).
They settle the filesystem and packaging questions left open in section 7 of
`docs/research/windows-tooling.md` on branch `research/windows-tooling`. This
file records what was seen on a GitHub Actions `windows-2025` runner. It
decides nothing.

Terms used throughout:

- **Launcher**: `climon2.exe`, the small program a user starts. **`c2.exe`** is
  its short second name, either a hard link to the same file or a separate
  copy.
- **Version folder**: a folder such as `0.1.0\` that holds one release's
  programs and libraries. **`current`** is a one-line text file naming the
  version folder in use.
- **Running image**: an `.exe` or `.dll` file that a live process has loaded.
- **Rename aside**: rename a file or folder to a spare name in the same folder
  (`climon2.exe` to `climon2.exe.old-1`). **Rename over**: rename a new file to
  a name that already exists, replacing what was there.
- **std rename / std delete**: Rust 1.96.0 `std::fs::rename` and
  `std::fs::remove_file`. `std::fs::rename` calls
  `MoveFileExW(MOVEFILE_REPLACE_EXISTING)` and, only when that returns error 5,
  retries with a POSIX-semantics rename (`FileRenameInfoEx`).
- **Raw rename / raw delete**: `MoveFileExW(MOVEFILE_REPLACE_EXISTING)` and
  `DeleteFileW` called directly, with no retry.
- **Mark-of-the-Web**: the `Zone.Identifier` alternate data stream that marks a
  file as downloaded from the internet.

Error codes are Win32 codes: 2 `ERROR_FILE_NOT_FOUND`, 5
`ERROR_ACCESS_DENIED`, 32 `ERROR_SHARING_VIOLATION`, 145
`ERROR_DIR_NOT_EMPTY`.

The probe code is in `probes/windows-install/` and the workflow is
`.github/workflows/windows-install-probes.yml`. Raw output is in
`probes/windows-install/results/`, one folder per workflow run. Unless a
section says otherwise, every run gave the same result and the numbers quoted
are from `run-4`.

## Surprises

Each item contradicts the documentation quoted in `windows-tooling.md` or an
expectation in the probe brief.

1. **A running image with a second hard link is not protected (probe 2).** When
   `c2.exe` is a hard link to `climon2.exe`, either name can be deleted or
   renamed over while a process runs from either name, with std and raw calls
   alike. The process stays healthy. Only the last remaining name is protected:
   deleting it, or renaming over it, fails with 5.
2. **A folder holding a running image can be renamed (probe 4).** A folder
   containing a running `.exe`, or a `.dll` loaded by a process, renames aside
   and a new folder renames into its place. The process stays healthy. The
   rename fails only when a process has an ordinary open file handle inside the
   folder (5) or has the folder as its working directory (32).
3. **Replacing `current` with std rename is not seamless for readers on NTFS
   (probe 5).** Over 20 s of back-to-back replacements, readers saw the path
   as missing (2) in bursts lasting 47 to 106 ms, 0 to 2 bursts per 20 s. Both
   readers saw the same burst at the same moment. Two single reads across
   all runs failed with 5 instead. The writer reported no failure. No reader ever saw
   empty or partial content. ReFS showed no reader failures.
4. **Raw rename over `current` fails most of the time under reading (probe
   5).** With two readers looping, 86 to 93 percent of raw
   `MoveFileExW(REPLACE_EXISTING)` calls failed with 5. Readers saw no failures
   at all in that mode. std rename never failed for the writer.
5. **ReFS lets std rename replace a running image (probes 1 and 3).** On ReFS,
   `std::fs::rename` over a running `.exe` with no other link succeeds, and the
   old process stays healthy. On NTFS the same call fails with 5. Raw rename
   fails with 5 on both.
6. **Both runner volumes are NTFS.** `D:` was expected to be ReFS. The ReFS
   results come from a 20 GB virtual disk the workflow created and formatted
   (ReFS 3.14).
7. **Only Explorer's extraction carries Mark-of-the-Web (probe 8).**
   `Expand-Archive` in Windows PowerShell 5.1 and in pwsh 7, `tar.exe` and
   .NET `ZipFile` all extract unmarked files from a marked zip. Explorer's zip
   extraction marks every extracted file.
8. **`[Environment]::SetEnvironmentVariable` changes the PATH type (probe
   10).** It rewrote the user PATH from `REG_EXPAND_SZ` to `REG_SZ` in both
   shells, so the `%USERPROFILE%` entries stopped expanding.
9. **A locked lock file can be renamed and deleted (probe 6).** While another
   process held the lock, the file was renamed aside and then deleted. A new
   file at the original path then locked successfully, so two processes each
   believed they held the lock.

## Environment

| Item | Value |
| --- | --- |
| Runner | GitHub-hosted `windows-2025`, `ImageOS=win25-vs2026`, `ImageVersion=20261116.315.1`, x64 |
| OS | Windows Server 2025 Datacenter 24H2, build 26100.33828. Server, not client. |
| User | `runneradmin`, elevated |
| Defender | Service on, real-time and on-access protection off |
| Rust | 1.96.0 (ac68faa20 2026-05-25), `x86_64-pc-windows-msvc`, from the repository's `rust-toolchain.toml` |
| MSVC | Visual Studio Enterprise 18.12.0, tools 14.52.37025 |
| Shells | pwsh 7.6.10; Windows PowerShell 5.1.26100.33822 |
| `C:` | NTFS, 161 GB. `%LOCALAPPDATA%` is `C:\Users\runneradmin\AppData\Local`. |
| `D:` | NTFS. Two machine shapes appeared: "Temporary Storage", 161 GB (runs 1, 2 attempt 2, 3) and "Temp", 236 GB (runs 2 attempt 1, 4). |
| `R:` | ReFS 3.14 on a 20 GB virtual disk created by `scripts/refs.ps1`; reports "Supports Hard Links", "Supports Named Streams", "Supports POSIX-style Unlink and Rename" |
| Long paths | `LongPathsEnabled=1` |

Runs, all on branch `research/windows-install-probes`:

| Folder | Workflow run | Commit | What it added |
| --- | --- | --- | --- |
| `run-1` | [44911781994](https://github.com/jackgeek/climon2/actions/runs/44911781994) | c7bc150b | First run; `C:` and `D:`, std calls with raw calls only after a failure |
| `run-2-attempt-1`, `run-2-attempt-2` | [44912023307](https://github.com/jackgeek/climon2/actions/runs/44912023307) | 95ec3d35 | Raw-only pass of probes 1 to 4; ReFS volume |
| `run-3` | [44912623709](https://github.com/jackgeek/climon2/actions/runs/44912623709) | 7f8a1deb | Probe 5 race lengthened from 3 s to 20 s |
| `run-4` | [44913098989](https://github.com/jackgeek/climon2/actions/runs/44913098989) | 4b9e5f71 | Probe 5 reader records failure bursts |

Probes 1 to 6 and 9 ran on `C:` under `%LOCALAPPDATA%`, on `D:` and on `R:`.
Results on `C:` and `D:` were identical in every step. In the JSON files,
probes named `1` to `4` use std calls and record a raw call only after the std
call failed; probes named `1-raw` to `4-raw` use raw calls only.

In every case below "healthy" means the process had not exited and kept
writing a counter to a heartbeat file every 50 ms.

## Probe 1: rename a running exe aside

One `climon2.exe` with no other link, started from the test folder.

| Step | NTFS (`C:`, `D:`) | ReFS (`R:`) |
| --- | --- | --- |
| (a) Rename running exe to `climon2.exe.old-1` | Succeeds, std and raw. Process healthy. | Same |
| (b) Rename a different exe into the original name, then run it | Succeeds; the new program runs and exits 0; the old process stays healthy | Same |
| Rename the aside file a second time (`.old-1` to `.old-2`) | Succeeds | Same |
| (c) Delete the aside file while its process runs | Fails: std 5, raw 5 | Same |
| (c) Delete the aside file after the process exited | Succeeds on the first attempt, 0 ms after `wait` returned | Same (raw pass; see note) |
| Delete the running exe under its original name | Fails: std 5, raw 5 | Same |
| (d) Rename a new file over the running exe | Fails: std 5, raw 5 | **std succeeds**; raw fails with 5 |

Note on ReFS: in the std pass the "rename over the aside image" step
succeeded, so the aside name then held the new file. The later delete steps of
that pass acted on the new file and say nothing about a running image. The
single-step cases and the raw pass are unaffected.

## Probe 2: hard-link sibling

`c2.exe` is a hard link to `climon2.exe`. Each row was run in a fresh folder
with a fresh process, for all four combinations of running name and target
name, with std calls and again with raw calls. All eight combinations per
operation gave the same result on NTFS and ReFS.

| Operation on either name, process running from either name | Result |
| --- | --- |
| Delete | Succeeds. The other name remains. Process healthy. |
| Rename aside, then delete the aside name | Both succeed |
| Rename a new file over it, then run the new file | Succeeds; the new program runs; the old process stays healthy |

Replacing both names while the old image runs, tested with a process running
from `climon2.exe` only and again with one process running from each name:

| Step | Result |
| --- | --- |
| Rename `climon2.exe` aside, rename `c2.exe` aside | Both succeed |
| Rename a new `climon2.exe` in, hard-link a new `c2.exe` to it, run both | All succeed; old processes stay healthy |
| Delete the first aside name while the old image runs | Succeeds |
| Delete the second (last) aside name while the old image runs | Fails: std 5, raw 5 |
| Delete the last aside name after the processes exited | Succeeds on the first attempt |

## Probe 3: copy sibling

`c2.exe` is an independent copy of `climon2.exe`.

| Operation | Result |
| --- | --- |
| Only `climon2.exe` running: rename a new file over `c2.exe` | Succeeds, std and raw; the new `c2.exe` runs |
| Only `climon2.exe` running: delete `c2.exe`, or rename it aside and delete it | Succeeds |
| Only `c2.exe` running: the same three operations on `climon2.exe` | Succeed |
| Any operation on the name that is running | As probe 1: rename aside succeeds; delete fails with 5; rename over fails with 5 on NTFS (std succeeds on ReFS) |

## Probe 4: directory rename

A folder `0.1.0\` in five states. "Rename in" means renaming a prepared
`0.1.0.new\` to `0.1.0` after the old folder was renamed aside. Results were
the same on NTFS and ReFS.

| Folder contains | Rename folder aside | Rename new folder in | Process after |
| --- | --- | --- | --- |
| (a) A running exe | Succeeds, std and raw | Succeeds | Healthy |
| (b) A DLL loaded by a process whose exe is elsewhere | Succeeds, std and raw | Succeeds | Healthy |
| (c) Only idle files | Succeeds, std and raw | Succeeds | n/a |
| A file another process holds open for reading (Rust default share mode) | Fails: std 5, raw 5. Succeeds after the process exits. | Not reached | n/a |
| Nothing open, but it is a process's working directory | Fails: std 32, raw 32. Succeeds after the process exits. | Not reached | Healthy |

Renaming a new folder directly over an existing folder:

| Existing folder | std rename | Raw rename |
| --- | --- | --- |
| Not empty (any of the states above) | Fails with 145 (5 when it is a process's working directory) | Fails with 5 |
| Empty | Succeeds | Fails with 5 |

## Probe 5: `current` pointer file

The writer writes `current.tmp` and renames it over `current`, with no retry.

**(a) Another process holds `current` open for reading** (`File::open`, Rust
default share mode). The std rename over it succeeded, a raw rename over it
succeeded, and 100 further std replacements all succeeded. Afterwards the path
read `v102`, and the old handle still read the original `v0`. Same on all three
volumes.

**(b) Two reader processes open and read `current` in a tight loop** while the
writer replaces it back-to-back for 20 s (`run-4`; `run-3` is the same test
without burst timing; runs 1 and 2 ran it for 3 s).

| Volume | Writer call | Replacements | Writer failures | Reads | Reader failures |
| --- | --- | --- | --- | --- | --- |
| `C:` NTFS | std | 22,578 | 0 | 665,280 | 23,889, all `open` error 2, in one burst of 106 ms in one reader |
| `C:` NTFS | raw | 47,683 | 40,930 (error 5) | 843,904 | 0 |
| `D:` NTFS | std | 63,331 | 0 | 446,656 | 22,027 `open` error 2 and 1 `open` error 5 |
| `D:` NTFS | raw | 128,162 | 119,274 (error 5) | 490,752 | 0 |
| `R:` ReFS | std | 46,533 | 0 | 313,600 | 0 |
| `R:` ReFS | raw | 125,870 | 114,663 (error 5) | 396,096 | 0 |

Detail of the `D:` std failures in `run-4`: both readers saw the path missing
at the same time, 4.27 s in, for 47 ms (10,966 and 10,742 consecutive failed
opens), and again 17.6 s in, for 1.4 ms. One reader had a single error 5 at
5.8 s.

Across all runs:

- No read ever returned empty or partial content. Every failure was a failed
  `open`.
- std rename never failed for the writer (about 260,000 replacements).
- Reader failures with std rename on NTFS: none in the six 3-second runs
  except a single error 5 (`run-2-attempt-2`, `D:`); in the four 20-second
  NTFS runs, three had bursts of error 2 (`run-3` `C:`, `run-4` `C:` and
  `D:`) and one had none (`run-3` `D:`).
- Reader failures with std rename on ReFS: none in four runs.
- Reader failures with raw rename: none on any volume, while 86 to 93 percent
  of the writer's renames failed with 5.

## Probe 6: lock file

`File::try_lock` on `home\install.lock`, held by a separate process. Same on
all three volumes.

| Step | Result |
| --- | --- |
| Second process calls `try_lock` | `WouldBlock` |
| 1,000 write-temp-then-rename-over replacements of another file in the lock's folder while the lock is held | All succeed; a second process still gets `WouldBlock` afterwards |
| Rename the lock's parent folder aside while the lock is held | Fails: std 5, raw 5 |
| Kill the holder with `taskkill /F`, then `try_lock` | Locked on the first attempt, 39 to 100 ms after `taskkill` was started (14 measurements) |
| Rename the lock file itself aside while it is locked | Succeeds |
| `try_lock` on the original path after that (creates a new file) | Locked |
| `try_lock` on the aside name (the file still held) | `WouldBlock` |
| Delete the aside lock file while it is still locked | Succeeds |

## Probe 7: CRT imports

`dumpbin /dependents`, MSVC tools 14.52.37025. The Rust artifacts are the
probe's own release builds: an `.exe` and a `cdylib` that uses std formatting
and allocation.

| File | `vcruntime140*.dll` / `msvcp140*.dll` | Other imports |
| --- | --- | --- |
| Rust exe, default flags | **`VCRUNTIME140.dll`** | `KERNEL32.dll`, `ntdll.dll`, `api-ms-win-core-synch-l1-2-0.dll`, five `api-ms-win-crt-*` (UCRT) DLLs |
| Rust cdylib, default flags | **`VCRUNTIME140.dll`** | `KERNEL32.dll`, `ntdll.dll`, `api-ms-win-core-synch-l1-2-0.dll`, two `api-ms-win-crt-*` DLLs |
| Rust exe, `-C target-feature=+crt-static` | None | `KERNEL32.dll`, `ntdll.dll`, `api-ms-win-core-synch-l1-2-0.dll` |
| Rust cdylib, `+crt-static` | None | The same three |
| `conpty.dll` x64 | None | `api-ms-win-core-*` and five `api-ms-win-crt-*` DLLs |
| `OpenConsole.exe` x64 | None | `api-ms-win-*`, seven `api-ms-win-crt-*`, `ntdll.dll`, `USER32`, `GDI32`, `SHELL32`, `OLEAUT32`, `PROPSYS`, `WINMM`, `IMM32`, `USP10`, `d2d1`, `DWrite`, `dxgi`, `d3d11`, `D3DCOMPILER_47`, `icu.dll`, two `ext-ms-win-uiacore-*` |

Nothing imports `msvcp140*.dll`. `+crt-static` grew the exe from 414 KB to
510 KB and the cdylib from 109 KB to 203 KB.

The ConPTY files come from `Microsoft.Windows.Console.ConPTY` 1.25.261109001,
the newest stable version on nuget.org; `origin/main` names the package but
pins no version. The `.nupkg` SHA-256 is
`3D77F535DF848E44225C2A8912AC844FECB23F3057732B639F3034B587496F09`.

The runner has `vcruntime140.dll` 14.52.37029.0 in `System32`, so it cannot
show what happens on a machine without it.

## Probe 8: Mark-of-the-Web propagation

A zip holding `climon2.exe` and `install.ps1` was given a `Zone.Identifier`
stream (`[ZoneTransfer]`, `ZoneId=3`) and extracted under `%LOCALAPPDATA%` on
`C:`.

| Extractor | Extracted files carry `Zone.Identifier` |
| --- | --- |
| Windows PowerShell 5.1 `Expand-Archive` (Archive module 1.0.1.0) | No |
| pwsh 7.6.10 `Expand-Archive` (Archive module 1.2.7) | No |
| `tar.exe -xf` (bsdtar 3.8.9) | No |
| .NET `ZipFile.ExtractToDirectory` | No |
| Explorer's zip extraction, driven through `Shell.Application` `CopyHere` | **Yes**: `ZoneId=3` and `ReferrerUrl=<path of the zip>` on both files |

Downloads of one file from `api.nuget.org`:

| Downloader | File carries `Zone.Identifier` |
| --- | --- |
| Windows PowerShell 5.1 `Invoke-WebRequest -UseBasicParsing -OutFile` | No |
| Windows PowerShell 5.1 `Invoke-RestMethod -OutFile` | No |
| pwsh 7 `Invoke-WebRequest -OutFile` | No |
| `curl.exe -o` | No |
| `Start-BitsTransfer` | No |

## Probe 9: hard links by volume

| Volume | `std::fs::hard_link` (`CreateHardLinkW`) | Alternate data stream |
| --- | --- | --- |
| `C:` NTFS, under `%LOCALAPPDATA%` | Works | Works |
| `D:` NTFS | Works | Works |
| `R:` ReFS 3.14 | Works | Works |

All of probe 2 also passed on ReFS with the same results as NTFS.

## Probe 10: user PATH registry type

`HKCU\Environment\Path` on the runner is **`REG_EXPAND_SZ`**:

```
%USERPROFILE%\.dotnet\tools;%USERPROFILE%\.cargo\bin;%USERPROFILE%\AppData\Local\Microsoft\WindowsApps;
```

The machine PATH under `HKLM` is `REG_SZ`. Windows PowerShell 5.1 and pwsh 7
gave identical output for every step.

| Step | Result |
| --- | --- |
| `(Get-Item HKCU:\Environment).GetValueKind('Path')` | `ExpandString` |
| `.GetValue('Path', $null, 'DoNotExpandEnvironmentNames')` | The unexpanded string above |
| `.GetValue('Path')` | The same with `%USERPROFILE%` expanded to `C:\Users\runneradmin` |
| Append `;%LOCALAPPDATA%\climon2-probe\bin` with `RegistryKey.SetValue('Path', $new, $originalKind)` | Kind stays `ExpandString`; the stored string equals what was written; the original text is an exact prefix |
| `[Environment]::SetEnvironmentVariable('Path', $new, 'User')` with the same unexpanded string | Kind becomes **`String`** (`REG_SZ`); the text is stored unchanged, so a normal read now returns literal `%USERPROFILE%` |
| Restore with `SetValue('Path', $original, $originalKind)` | Kind and text identical to the start |

## Not measured

- **A client edition of Windows.** The runner is Windows Server 2025. Nothing
  here shows SmartScreen or Smart App Control behaviour, or how a marked file
  behaves when started.
- **A machine without `vcruntime140.dll`.** The runner has it installed.
- **Explorer's interactive extraction.** The `Shell.Application` call uses the
  same shell code without a desktop; a user dragging files out of a zip window
  was not tested.
- **Real-time antivirus.** Defender's real-time protection is off on the
  runner. A scanner that opens new files could change the probe 1, 4 and 5
  results.
- **The cause of the probe 5 not-found bursts.** The probe records that they
  happen, not why.
