# Windows quality tooling, packaging and DLL loading

Research for [#210](https://github.com/jackgeek/climon2/issues/210), child of
[#205](https://github.com/jackgeek/climon2/issues/205). It informs the quality,
release/installer and monitor-loading decisions downstream (#214, #215, #216). It
states facts and options only. It decides no gate design, installer policy or
library-loading rule.

Scope as amended on 2026-11-14: **Windows 11 only** (x86_64 MSVC); Windows 10 is not
supported. GitHub's Windows runners are **Windows Server** editions, so every place
where a runner can differ from a user's Windows 11 machine is flagged **⚑SERVER**.

Baseline: `origin/main` at `ed94bf1`. Rust `1.96.0` (`rust-toolchain.toml`), whose
rustc uses LLVM 22.1.2. Researched on the Linux development host. Nothing was built for
Windows, and no Windows experiment ran, because CI is the only Windows environment.

Labels: **[V]** read directly from a pinned source, a primary doc or a local artifact;
**[D]** derived from cited sources, not executed. Shorthand: `MSL` =
`https://learn.microsoft.com/en-us/windows/win32/`, `RR` =
`https://github.com/rust-lang/rust/blob/1.96.0/`.

## 1. Repository starting point (`origin/main`)

- **[V]** CI's only Windows job is `neutral` on `windows-latest`, which runs
  `cargo test --locked -p climon2-core` (`.github/workflows/check.yml:67-82`). `quality`
  runs on `macos-15` only (`check.yml:83-125`). `dist` and `archive` cover
  `macos-universal` and `linux-x86_64-gnu` (`check.yml:126-158,159-252`). `publish`
  runs on `macos-15`, compares two `install.sh` copies with `cmp`, merges two
  `SHA256SUMS`, checks them with `shasum`, and uploads two named archives
  (`.github/workflows/release.yml:15-51`).
- **[V]** Pins: crapper `6fd92c4c…`, mutator `a5a0cb80…`, cargo-llvm-cov `0.9.1`, CRAP
  threshold 5 (`xtask/src/quality.rs:15-31`). The registered native files are four
  Linux and four macOS paths; there is no `windows` arm (`quality.rs:476-488`).
- **[V]** The installer and every runtime platform seam end in `compile_error!` off
  macOS and Linux (`crates/climon2-install/src/platform.rs:1-2`; ADR 0003).

## 2. Quality tooling on x86_64 MSVC

### 2.1 LLVM coverage: instrumentation, spawned and killed processes

- **[V]** cargo-llvm-cov says `{x86_64,i686}-pc-windows-msvc` "are confirmed to work"
  ([README @ v0.9.1, Platform Support › Windows](https://github.com/taiki-e/cargo-llvm-cov/blob/v0.9.1/README.md#windows)).
  Its old `-C codegen-units=1` Windows workaround applies only to the nightly `-Z`
  path, not to stable `-C instrument-coverage`
  ([`src/main.rs` L154-172](https://github.com/taiki-e/cargo-llvm-cov/blob/v0.9.1/src/main.rs#L154-L172)).
- **[V]** The LLVM 22.1.2 docs still say `%c` (continuous mode) "is only supported on
  Darwin at the moment ... support for Windows may require more extensive changes"
  ([`SourceBasedCodeCoverage.rst` L102-109](https://github.com/llvm/llvm-project/blob/llvmorg-22.1.2/clang/docs/SourceBasedCodeCoverage.rst#L102-L109)).
- **[V]** The runtime source disagrees with that doc. `_WIN32` shares the ELF branch
  (`#elif defined(__ELF__) || defined(_WIN32) || defined(_AIX)`), which sets
  `ContinuousModeSupported = 1` and `UseBiasVar = 1`. Under MSVC it uses
  `/alternatename` in place of a weak alias to detect `__llvm_profile_counter_bias`
  ([`InstrProfilingFile.c` L205-241 @ llvmorg-22.1.2](https://github.com/llvm/llvm-project/blob/llvmorg-22.1.2/compiler-rt/lib/profile/InstrProfilingFile.c#L205-L241)).
  The `%c` parser enables continuous mode for `_WIN32` ("Mach-O, ELF and COFF formats",
  L854-861).
- **[V]** The mapping goes through the runtime's POSIX shim: `mmap` becomes
  `CreateFileMapping` plus `MapViewOfFile`, and `flock` becomes `LockFileEx`
  ([`WindowsMMap.c` L28-77,157-182](https://github.com/llvm/llvm-project/blob/llvmorg-22.1.2/compiler-rt/lib/profile/WindowsMMap.c#L28-L77)).
- **[V]** Without the bias symbol, initialisation prints "Neither
  `__llvm_profile_counter_bias` nor `__llvm_profile_bitmap_bias` is defined" and returns
  (`InstrProfilingFile.c` L631-640). The compiler emits it only with
  `-runtime-counter-relocation`, which it honours on every object format except Mach-O
  ([`InstrProfiling.cpp` L844-855](https://github.com/llvm/llvm-project/blob/llvmorg-22.1.2/llvm/lib/Transforms/Instrumentation/InstrProfiling.cpp#L844-L855)).
- **[V]** `xtask/src/coverage.rs:49-58` adds `-Cllvm-args=-runtime-counter-relocation`
  only when `target_os = "linux"`. On every other OS it adds the Darwin
  `-Wl,-sectalign,__DATA,…` link arguments.
- **[D]** As written, a Windows coverage run would therefore:
  1. pass Darwin `ld64` flags to `link.exe`;
  2. omit counter relocation, so `%c` would silently do nothing beyond the error line;
  3. lose counters from any process ended by `TerminateProcess`.

  This matters for the hang-up grace path in ADR 0003's Windows notes, because
  TerminateProcess skips exit handlers the same way SIGKILL does.

  A Windows arm would take the ELF-style relocation flag. The COFF continuous path has
  no upstream claim of testing, so a coverage-under-`TerminateProcess` probe on the
  runner is the only evidence available. Nothing was executed here.
- **[V]** The ELF/COFF branch carries a TODO: "If there are two DSOs, the second DSO
  initialization will truncate the first profile file" (`InstrProfilingFile.c`
  L242-243). `%m` keys files by binary signature, so an instrumented
  `climon2_client.dll` and its host write separate pools. **[D]**
- **[D]** "Exec-replaced" has no Windows meaning. The launcher spawns and waits
  (ADR 0003 notes), so the `tooling-probe` contract of "one normal plus one
  exec-replaced run gives two hits" needs a Windows equivalent: spawned, and spawned
  then terminated.
- **[V]** The `show-env` default format escapes values with `shell_escape::escape`
  (`cli.rs` L715-745 @ v0.9.1). On Windows that is cmd-style double-quoting when a
  value contains space, tab, newline or `"`, unless `MSYSTEM` is set (as in Git Bash),
  where it is POSIX single quoting (`shell-escape-0.1.5/src/lib.rs:17-23,35-60`).
  `coverage.rs:24-35` strips only single quotes. **[D]** A Windows path containing
  spaces would be mis-parsed outside Git Bash. Runner workspaces such as `D:\a\…` have
  none.
- **[V]** Cargo applies only the first of `CARGO_ENCODED_RUSTFLAGS`, `RUSTFLAGS`,
  `target.<triple>.rustflags`, `build.rustflags`
  ([cargo config reference, `build.rustflags` @ rust-1.96.0](https://github.com/rust-lang/cargo/blob/rust-1.96.0/src/doc/src/reference/config.md#buildrustflags)).
  **[D]** If the static CRT is chosen through `target.x86_64-pc-windows-msvc.rustflags`
  (§5.3), the coverage `RUSTFLAGS` (`coverage.rs:66`) would silently drop it for
  instrumented builds. `release::build` removes `RUSTFLAGS` (`xtask/src/release.rs:29-30`),
  so release builds would keep it, and coverage and release binaries would then differ
  in CRT linkage.

### 2.2 Pinned crapper and mutator on Windows

Sources: [`unclebob/crapper@6fd92c4c`](https://github.com/unclebob/crapper/tree/6fd92c4c9bcc56efb64a2f8743d5a2993bcfd42d),
[`unclebob/mutator@a5a0cb80`](https://github.com/unclebob/mutator/tree/a5a0cb803b66e0cdb8bf471b3b08af701d424a41).

- **[V]** Both are pure Python. The two native requirements publish `win_amd64` wheels:
  `tree-sitter==0.26.0` for cp310–cp314, and `tree-sitter-language-pack==1.22.1` as
  `cp310-abi3-win_amd64` (PyPI JSON API, queried 2026-11-14).
- **[V]** crapper normalises LCOV paths by turning `\` into `/` (`coverage.py:40-48`),
  and both tools compare `as_posix()` forms. **[D]** Drive-letter LCOV paths from
  llvm-cov should match. Not executed.
- **[V]** Blockers in the mutator for `--native-only` on Windows:
  1. Timeout handling calls `os.killpg(process.pid, signal.SIGKILL)`
     (`runner.py:109-110`). `os.killpg` is "Availability: Unix", and `signal.SIGKILL`
     is Unix-only
     ([os.rst](https://github.com/python/cpython/blob/3.13/Doc/library/os.rst),
     [signal.rst](https://github.com/python/cpython/blob/3.13/Doc/library/signal.rst)).
     **[D]** The first mutant that times out raises `AttributeError` rather than
     counting as killed. Mutants that finish are unaffected.
  2. `start_new_session=True` is documented "Availability: POSIX"
     ([subprocess.rst L595-600](https://github.com/python/cpython/blob/3.13/Doc/library/subprocess.rst)).
     **[D]** It is ignored on Windows, so there is no process-group kill: grandchildren
     (cargo, rustc, test binaries, daemons) survive a timed-out parent unless the test
     command uses a job object.
  3. A string `--test-command` runs with `shell=True` (`runner.py:92-95`), which on
     Windows uses `%COMSPEC%`, cmd.exe (subprocess.rst L457). climon2's
     `TEST_COMMAND` is `"$CLIMON2_MUTATION_PYTHON" tools/test-mutation.py`
     (`quality.rs:39`). **[D]** cmd.exe does not expand `$VAR` (its syntax is
     `%VAR%`), so every mutant would fail to start and could be scored as killed.
     That makes it a false-kill hazard, not merely a crash.
  4. `_project_interpreter` looks only for `bin/python` (`runner.py:155-165`). This
     does not matter here, because climon2 supplies its own test command.
- **[V]** Worker overlays are symlinks (`workers.py:89-93`, `Path.symlink_to`). On
  Windows that needs Developer Mode, `SeCreateSymbolicLinkPrivilege` or administrator
  rights (os.rst, `symlink`). **⚑SERVER:** runners enable Developer Mode
  (`AllowDevelopmentWithoutDevLicense=1`,
  [`Configure-DeveloperMode.ps1` @ 95e6f51](https://github.com/actions/runner-images/blob/95e6f51a0685/images/windows/scripts/build/Configure-DeveloperMode.ps1))
  and "run as administrators with User Account Control (UAC) disabled"
  ([GitHub-hosted runners reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)).
  Symlink overlays should therefore work in CI and need not work on a default
  Windows 11 user machine.
- **[V]** `Path.symlink_to` creates a file or directory link matching an existing
  target (pathlib.rst L1494-1502). CPython's `unlink` removes directory symlinks and
  junctions through `RemoveDirectoryW` (`Py_DeleteFileW`,
  [posixmodule.c @ 3.13](https://github.com/python/cpython/blob/3.13/Modules/posixmodule.c)).
  The mutator's `delete_tree` does not follow links, so it works. **[D]**
- **[V]** A deep Cargo `target/` inside each worker is subject to Windows file locking:
  "attempting to remove a file that is in use causes an exception" (os.rst, `remove`).

### 2.3 climon2's own quality code and scripts

- **[V]** `xtask` hard-codes the POSIX venv layout `venv/bin/python` (`quality.rs:261,295,307,321,678`).
  On Windows, venv creates `Scripts` and `python.exe`
  ([venv.rst L74,217](https://github.com/python/cpython/blob/3.13/Doc/library/venv.rst)).
  The default interpreter is `python3` (`quality.rs:252`), with the
  `CLIMON2_QUALITY_PYTHON` override.
- **[V]** `tools/test-mutation.py:59-64` seeds worker caches with
  `cp -a --reflink=auto`, and on macOS with `cp -cRp`. Windows has no `cp` on the
  default cmd/Python `PATH`. **[D]** Without the seed it falls back to a cold build
  ("building a cold mutation worker").
- **[V]** `tools/check-mutation-load.py` uses `start_new_session`, `os.killpg`,
  `signal.SIGKILL` and `signal.SIGHUP` (`:56,85,191`). `signal.SIGHUP` is
  "Availability: Unix", so it would fail at startup. **[V]** It runs only for the
  `Ticket` and `Full` scopes. `Native`, which is `--native-only`, measures no load
  (`quality.rs:76-80`; distribution.md "does not measure load"). It therefore does not
  block a Windows `--native-only` gate.
- **[V]** `Scope::Native` uses `available_parallelism().min(16)` workers
  (`quality.rs:66-71`).
- **[V]** Most `xtask` quality unit tests are `#[cfg(unix)]` and use `#!/bin/sh` fake
  tools (`quality.rs:1060-1400`), so they do not exercise a Windows path.
- **[V]** The Linux research found crapper cfg-blind: an OS's out-of-line native file
  has no LCOV records on the other OS and scores 0%
  ([linux-quality-release.md §3](https://github.com/jackgeek/climon2/blob/be10464f71839897a269035a6e298b9836316969/docs/research/linux-quality-release.md)).
  **[D]** Windows-only bodies inside shared files would be scored by the Linux host
  gate as uncovered. Registered native files avoid that, as for macOS.

### 2.4 Runner images and wall-clock

- **[V]** `windows-latest` and `windows-2025` both map to "Windows Server 2025 with
  Visual Studio 2026". `windows-2022` is Server 2022. The only Windows 11 image is
  `windows-11-arm`, which is arm64
  ([runner-images README @ 95e6f51](https://github.com/actions/runner-images/blob/95e6f51a0685/README.md)).
  **⚑SERVER:** there is no x64 Windows 11 hosted image, so every x64 CI result comes
  from Server 2025 (build 26100, the same kernel generation as Windows 11 24H2) or
  Server 2022 (build 20348).
- **[V]** `windows-latest` moves gradually to the newest image ("-latest migration
  process", same README). The Linux precedent pinned `ubuntu-24.04` for the same reason.
- **[V]** Server 2025/VS 2026 image 20261103: Rust 1.99.1, rustup 1.29.3, Python
  3.12.10, Git 2.56.0, Git Bash 5.3, PowerShell 7.6.9, LLVM 20.1.8, VS Enterprise 2026,
  and VC++ 2022 x64 runtime 14.51
  ([`Windows2025-VS2026-Readme.md`](https://github.com/actions/runner-images/blob/95e6f51a0685/images/windows/Windows2025-VS2026-Readme.md)).
  rustup honours the repo's `1.96.0` pin.
- **[V] ⚑SERVER:** the image build disables Defender real-time monitoring, behaviour
  monitoring and IOAV scanning, and excludes `C:\` and `D:\`
  ([`Configure-WindowsDefender.ps1`](https://github.com/actions/runner-images/blob/95e6f51a0685/images/windows/scripts/build/Configure-WindowsDefender.ps1)).
  **[D]** CI therefore sees neither the AV-induced sharing violations nor the build
  slowdown that a Windows 11 user machine would. Any rename/replace retry logic in the
  installer cannot be proven by CI.
- **[V] ⚑SERVER:** from 2026-11-30 the runners' `D:` drive, which holds the
  workspace, is formatted ReFS instead of NTFS
  ([runner-images#15315](https://github.com/actions/runner-images/issues/15315)).
  `%LOCALAPPDATA%` stays on `C:`.
- **[V]** Standard public-repo Windows x64 runners have 4 CPUs, 16 GB RAM and 14 GB
  SSD. `macos-15` arm64 has 3 CPUs (M1) and 7 GB. A job may run "up to 6 hours"
  ([runners reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners);
  [limits](https://docs.github.com/en/actions/reference/limits)).
- **[V]** Measured precedent, from `gh run view` on the last 8 successful `Checks` runs
  on 2026-11-14:
  - The macOS `--native-only` quality job took **62–72 min** when native source
    changed, with "Qualify only macOS-exclusive source" at 69 min
    ([run 43611753497](https://github.com/jackgeek/climon2/actions/runs/43611753497/job/132498106264)).
    It took 2 min when skipped.
  - The Windows `neutral` job took 0–1 min.
  - Full macOS runtime tests took 15–23 min.
- **[D]** There is no Windows measurement. `--native-only` still runs whole-workspace
  coverage and the full-workspace worker baseline before mutating (distribution.md),
  so the cost scales with the full Windows test suite, not with the size of the native
  files.
  - With 4 workers on 4 vCPUs and Defender off, the order of magnitude is the macOS
    figure. It is unknown until measured, and well under the 6 h limit unless
    `--mutate-all` (as used on release tags, `release.yml:14`) multiplies it.
  - Losing the `cp` reflink seed makes every worker a cold build (§2.3).

## 3. Release artifacts in `xtask dist` and the release workflow

- **[V]** `host_platform` accepts only arm64 macOS and x86_64 GNU/Linux
  (`xtask/src/dist.rs:22-36`). Packaging copies `release/<binary>` with no suffix
  (`dist.rs:131`). Cargo's Windows outputs carry `.exe`
  (`std::env::consts::EXE_SUFFIX`). The archive uses `tar -czf` (`dist.rs:86`),
  checksums use `shasum` or `sha256sum` (`dist.rs:113-121`), and the stamped script
  must contain one `VERSION=` line (`dist.rs:235-256`).
- **[V]** PE verification facts
  ([PE format](https://learn.microsoft.com/en-us/windows/win32/debug/pe-format)):
  - "At location 0x3c, the stub has the file offset to the PE signature", which is
    `PE\0\0`.
  - COFF `Machine` is the first 2 bytes after it, and `IMAGE_FILE_MACHINE_AMD64` is
    `0x8664`.
  - Optional-header magic `0x20b` is PE32+.
  - Characteristics `IMAGE_FILE_EXECUTABLE_IMAGE 0x0002` and `IMAGE_FILE_DLL 0x2000`.
  - Subsystem `WINDOWS_CUI = 3`.
  - DllCharacteristics `HIGH_ENTROPY_VA 0x20`, `DYNAMIC_BASE 0x40`, `NX_COMPAT 0x100`,
    `GUARD_CF 0x4000`.

  **[D]** That is enough for a byte check in the shape of `verify_linux_binary`
  (`dist.rs:158-167`), including "is a DLL" for the planned client library.
- **[V]** On MSVC, `-C split-debuginfo=packed` is the default and writes a separate
  `*.pdb` (`unpacked` and `off` are unsupported)
  ([codegen options @ 1.96.0](https://github.com/rust-lang/rust/blob/1.96.0/src/doc/rustc/src/codegen-options/index.md#split-debuginfo)).
  The workspace defines no `[profile.release]`. **[D]** The `.pdb` files sit beside the
  `.exe` files in `target/…/release`. The archive includes them only if they are copied
  explicitly.
- **[V]** Windows ships `tar` based on libarchive's bsdtar. `tar -caf archive.zip
  .\folder` creates a zip, and `-a` picks the format from the extension
  ([tar on Windows](https://learn.microsoft.com/en-us/windows/tar/)).
- **[D]** Zip-creation options:
  1. bsdtar `-caf` (same tool family as today);
  2. a Rust zip crate in `xtask` (no zip crate is in `Cargo.lock` today);
  3. PowerShell `Compress-Archive` or .NET `ZipFile`.
- **[D]** `sha2 0.10.9` is already in `Cargo.lock`. Whether `xtask` may depend on it
  is a policy question. Any option must emit the `<hex>  <name>` lines that the release
  job merges with `sort -u` and checks with `shasum -c` (`release.yml:30-36`).
  PowerShell `Get-FileHash` prints a table, not that format (§4.1).
- **[V]** `release.yml` `publish` names exactly two archives, `cmp`s exactly two
  `install.sh` copies and runs on `macos-15` (`release.yml:17-50`). **[D]** A third
  artifact needs:
  - the Windows `release-dist-*` artifact downloaded alongside;
  - the zip, `install.ps1` and the checksum lines merged in;
  - `install.ps1` added to the `gh release create` list.

  `install.sh` is produced by every dist job and compared with `cmp`. A Windows dist
  job would produce `install.ps1`. Whether it also produces `install.sh` is a choice.
  `actions/attest-build-provenance` attests whatever paths it is given, so it is
  unaffected by platform.
- **[V]** The `archive` job's "previous release" step greps for `-$PLATFORM.tar.gz`
  (`check.yml:218-238`). **[D]** A `.zip` platform needs its own pattern. The scope note
  "compatibility starts from the second Windows release" lines up with the existing
  "first platform release" branch.

## 4. Installation primitives

Today's Unix installer is relevant here (`crates/climon2-install/src`):

- It takes `flock(LOCK_EX|LOCK_NB)` on the home **directory** (`platform.rs:62-68`).
- It stages under `.climon2-install-stage`.
- It exchanges a same-version folder with different contents using
  `renameat2(RENAME_EXCHANGE)` or `renamex_np` (`platform.rs:40-48`, `publish.rs:53-72`).
- It renames the launcher and then `current` with `fs::rename`
  (`publish.rs:8-21`).
- It makes `c2` a relative symlink (`publish.rs:35-50`).

### 4.1 `install.ps1` via `irm … | iex`

- **[V] Invoke-WebRequest prompt.** Since the 2025-12-09 updates (CVE-2025-54100),
  Windows PowerShell 5.1 `Invoke-WebRequest` shows a "Script Execution Risk"
  confirmation unless `-UseBasicParsing` is passed.
  - It has "no built-in mechanism to automatically consent", so unattended scripts
    "could hang".
  - It applies to Windows 11 24H2/25H2 and Server 2016–2025
    ([KB5074596](https://support.microsoft.com/KB/5074596);
    [Invoke-WebRequest 5.1](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.utility/invoke-webrequest?view=powershell-5.1)).
  - The KB names only `Invoke-WebRequest`.
  - The 5.1 `Invoke-RestMethod` page gives the same advice: "use the `-UseBasicParsing`
    switch"
    ([Invoke-RestMethod 5.1](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.utility/invoke-restmethod?view=powershell-5.1)).
- **[V] `-UseBasicParsing` in PowerShell 6+.** It has "no effect" there, because all
  requests use basic parsing
  ([Invoke-WebRequest 7.5](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.utility/invoke-webrequest?view=powershell-7.5)).
- **[V] Progress bar.** It slows `iwr` and `Expand-Archive` by large factors in 5.1.
  This was fixed only in PowerShell 6.0; the workaround is
  `$ProgressPreference='SilentlyContinue'`
  ([PowerShell#2138](https://github.com/PowerShell/PowerShell/issues/2138)).
- **[V] TLS.**
  - On .NET Framework 4.7+, `ServicePointManager.SecurityProtocol` defaults to
    `SystemDefault`.
  - The Windows 11 column lists TLS 1.3 for apps targeting 4.6.2–4.8.1.
  - Apps targeting ≤4.6.1 get `SystemDefaultTlsVersions=0`.
  - Microsoft advises SystemDefault, or else explicit Tls12/Tls13
    ([.NET Framework TLS](https://learn.microsoft.com/en-us/dotnet/framework/network-programming/tls)).
  - **Unverified:** which framework version `powershell.exe` 5.1 targets. A `-bor Tls12`
    line is the conservative idiom.
- **[V] `Get-FileHash`.** It exists in 5.1, defaults to SHA256, and prints an uppercase
  hex `Hash` property
  ([Get-FileHash 5.1](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.utility/get-filehash?view=powershell-5.1)).
  **[D]** PowerShell's `-eq` on strings is case-insensitive.
- **[V] `Expand-Archive`.** In 5.1 it handles zip only, is built on
  `System.IO.Compression.ZipArchive` (2 GB per-file limit), and does not overwrite
  without `-Force`
  ([Expand-Archive 5.1](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.archive/expand-archive?view=powershell-5.1)).
- **[V] `Invoke-Expression` scope.** It runs "in the current scope", and the call
  operator is called "a safer method"
  ([Invoke-Expression](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.utility/invoke-expression?view=powershell-5.1)).
  `exit` "causes PowerShell to exit a script or a PowerShell instance"
  ([about_Language_Keywords](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_language_keywords?view=powershell-7.5#exit)).
  **[D]** Consequences under `irm | iex`:
  - A script-level `param()` binds nothing.
  - `exit` closes the user's interactive window.
  - Preference variables leak into the user's session.

  `& ([scriptblock]::Create((irm URL))) -Arg …` runs the text as a child-scope script
  block. That pattern is not cited from a primary doc.
- **[V] Execution policy.** It "isn't a security boundary", and governs loading
  configuration files and running scripts.
  - **⚑SERVER:** with every scope Undefined, the effective policy is **Restricted on
    Windows clients and RemoteSigned on Windows Server**
    ([about_Execution_Policies](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_execution_policies?view=powershell-7.5)).
  - **[D]** Piped `iex` evaluates a string, not a script file, so it is not stopped by
    the policy. A downloaded `install.ps1` run as a file would be, on a default
    Windows 11. CI on Server would not reveal this.
- **[V] Native exit codes.** `$PSNativeCommandUseErrorActionPreference` (7.4+) makes
  non-zero native exit codes honour `$ErrorActionPreference`
  ([about_Preference_Variables](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_preference_variables?view=powershell-7.5)).
  **[D]** It does not exist in 5.1, so `$LASTEXITCODE` must be checked after
  `install.exe`.
- **[V] Shells on runners.** GitHub Actions' `pwsh` is the default Windows shell, and
  `powershell` is "the PowerShell Desktop"
  ([workflow syntax, `defaults.run.shell`](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax)).
  **[D]** CI can therefore exercise both `irm | iex` hosts.

### 4.2 User PATH (HKCU) and `WM_SETTINGCHANGE`

- **[V] Storage and broadcast.** User variables live in `HKCU\Environment`. After a
  change, broadcast `WM_SETTINGCHANGE` with lParam `"Environment"` so "applications,
  such as the shell" pick it up. A user-defined variable is limited to 32,767
  characters
  ([Environment variables](https://learn.microsoft.com/en-us/windows/win32/procthread/environment-variables);
  [WM_SETTINGCHANGE](https://learn.microsoft.com/en-us/windows/win32/winmsg/wm-settingchange)).
  - The message goes to top-level windows.
  - With `HWND_BROADCAST`, `SendMessageTimeoutW` can wait up to timeout × windows.
    `SMTO_ABORTIFHUNG` skips hung receivers
    ([SendMessageTimeoutW](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendmessagetimeoutw)).
  - **[D]** Running consoles and terminals keep their old environment, and only newly
    launched processes see the change.
- **[V] .NET `Environment.SetEnvironmentVariable(…, User)`.**
  - It reads with `GetValue(variable)`, which expands the value.
  - It writes with `SetValue(variable, value)`, which "stores all string values as
    RegistryValueKind.String".
  - It broadcasts with a 1 s `SMTO_NORMAL` timeout
    ([Environment.Windows.cs L411-455 @ dotnet/runtime b48cbf5](https://github.com/dotnet/runtime/blob/b48cbf5646e9b0400d369223b98c544936df779b/src/libraries/System.Private.CoreLib/src/System/Environment.Windows.cs#L411-L455);
    [RegistryKey.SetValue](https://learn.microsoft.com/en-us/dotnet/api/microsoft.win32.registrykey.setvalue?view=net-9.0)).
  - **[D]** A read-modify-write through it turns `REG_EXPAND_SZ` into `REG_SZ` and
    freezes `%VAR%` entries. Only .NET (Core) was read; .NET Framework (5.1) is
    unverified.
- **[V] Raw reads.** `RegistryValueOptions.DoNotExpandEnvironmentNames`, or
  `RegGetValueW` with `RRF_NOEXPAND`, returns the unexpanded value
  ([RegistryValueOptions](https://learn.microsoft.com/en-us/dotnet/api/microsoft.win32.registryvalueoptions?view=net-9.0);
  [RegGetValueW](https://learn.microsoft.com/en-us/windows/win32/api/winreg/nf-winreg-reggetvaluew)).
- **[V] Prior art: rustup.**
  - It refuses non-string PATH values.
  - It writes `REG_EXPAND_SZ` ("per issue #261").
  - It deletes the value if it would become empty.
  - It broadcasts with `SMTO_ABORTIFHUNG`, 5000 ms
    ([rustup `self_update/windows.rs` L508-565, L1008 @ c3fb8fc](https://github.com/rust-lang/rustup/blob/c3fb8fc1524e6cc7a88ea9f6f0fa2d812d7f84e2/src/cli/self_update/windows.rs#L508-L565)).
- **[D] Idempotence factors.** No single primary doc states these.
  - Compare case-insensitively.
  - Normalise trailing `\`.
  - Compare both raw and expanded forms, e.g. `%LOCALAPPDATA%\climon2` vs
    `C:\Users\…\climon2`.
  - Ignore empty segments.
  - Leave every other entry byte-identical.
  - Handling `--dir` changes requires knowing which entry climon2 added earlier. That
    means a marker or record, which is a policy choice.
- **⚑SERVER:** Explorer is the main `WM_SETTINGCHANGE` consumer. The runner's
  non-interactive service session gives no evidence that a user's Explorer picked the
  change up. Only the registry value can be asserted in CI. **[D]**

### 4.3 `c2.exe`: hard link or copy

- **[V] `CreateHardLinkW` limits.**
  - Files only, and every link on one volume.
  - At most 1023 links per file.
  - "Flags, attributes, access, and sharing … operate on a per-file basis … if you open
    a file that does not allow sharing, another application cannot share the file by
    creating a new hard link"
    ([CreateHardLinkW](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-createhardlinkw)).
  - `std::fs::hard_link` calls `CreateHardLinkW`
    (`RR library/std/src/sys/fs/windows.rs#L1449-L1453`).
- **[D] Shared locks.** `c2.exe` and `climon2.exe` are one file, so a running instance
  under either name constrains both names. Replacing the launcher must handle both
  names together.
- **[V] ReFS: sources conflict.** The CreateHardLink page's technology table says
  ReFS **No**, while the current ReFS overview lists hard links as supported
  ([ReFS overview](https://learn.microsoft.com/en-us/windows-server/storage/refs/refs-overview)).
  **⚑SERVER:** this matters only if CI installs to `D:` (ReFS from 2026-11-30). The
  default `%LOCALAPPDATA%` is on `C:`.
- **[D] Copy.** A copy is independent: it is a separate image with its own lock and no
  volume constraint, at about twice the launcher's size.

### 4.4 Replacing a running launcher; deferred cleanup

- **[V] Deleting.** `DeleteFile` fails while other handles are open "as a memory-mapped
  file" without `FILE_SHARE_DELETE`
  ([DeleteFileW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-deletefilew)).
  POSIX-semantics disposition also returns `STATUS_CANNOT_DELETE` when "there is an
  existing mapped view to the file"
  ([FILE_DISPOSITION_INFORMATION_EX](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntddk/ns-ntddk-_file_disposition_information_ex)).
  **[D]** A running `.exe` cannot be deleted by any documented user-mode route.
- **[V] Renaming over.** "Even if ReplaceIfExists is set to TRUE, the rename operation
  will still fail if a file with the same name already exists and is … a currently
  executing file"
  ([FILE_RENAME_INFORMATION](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/ns-ntifs-_file_rename_information)).
  **[D]** A new launcher cannot be renamed *over* the running name, so the running file
  must first be renamed aside.
- **[V] Renaming aside: unconfirmed.** The same page says "A file cannot be renamed if
  it has any open handles" (batch-oplock exception). No Microsoft page found states
  that an image-mapped `.exe` can be renamed aside, which is the premise of the
  confirmed scope.
  - rustup's self-update avoids the question: it deletes its old exe after exit using a
    helper process holding a `FILE_FLAG_DELETE_ON_CLOSE` handle
    ([rustup L719-748](https://github.com/rust-lang/rustup/blob/c3fb8fc1524e6cc7a88ea9f6f0fa2d812d7f84e2/src/cli/self_update/windows.rs#L719-L748)).
  - **This is the first experiment a Windows CI probe must settle.** It must test both
    the launcher name and its hard-link sibling.
- **[V] Reboot-time moves.** `MOVEFILE_DELAY_UNTIL_REBOOT` needs administrators or
  LocalSystem, because it writes HKLM `PendingFileRenameOperations`
  ([MoveFileExW](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw)).
  It is unavailable to a per-user install.
- **[D] Deferred-cleanup options.**
  - Sweep renamed-aside files on the next install or the next launcher start.
  - A rustup-style delete-on-close helper.
  - Leave the files until a later run succeeds.

### 4.5 Version folder replacement without an exchange

- **[V] No directory exchange.**
  - `MoveFileExW(MOVEFILE_REPLACE_EXISTING)` reports an error when the destination is
    an existing directory (MoveFileExW).
  - `ReplaceFileW` is file-only
    ([ReplaceFileW](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-replacefilew)).
  - TxF is slated for possible deprecation, and Microsoft recommends alternatives
    ([Deprecation of TxF](https://learn.microsoft.com/en-us/windows/win32/fileio/deprecation-of-txf)).
  - ReFS has no transactions (ReFS overview).
  - **[D]** There is no documented atomic directory exchange on NTFS or ReFS.
- **[V] Rust `fs::rename` @ 1.96.0.**
  - It tries `MoveFileExW(…, MOVEFILE_REPLACE_EXISTING)` first.
  - Only on `ERROR_ACCESS_DENIED` does it retry with
    `SetFileInformationByHandle(FileRenameInfoEx, REPLACE_IF_EXISTS | POSIX_SEMANTICS)`
    (`RR library/std/src/sys/fs/windows.rs#L1311-L1380`).
  - Its docs say a directory may replace only an empty directory
    (`RR library/std/src/fs.rs#L2778-L2800`).
- **[V] Folders with open files.** "A directory cannot be renamed if it or any of its
  subdirectories contains a file that has open handles" (FILE_RENAME_INFORMATION).
  **[D]** A live session's `climon2-daemon.exe` pins its version folder, so any scheme
  that renames an in-use version folder fails. Options:
  - **(a)** rename aside, then rename in, with a missing-version window and failure
    when the folder is in use;
  - **(b)** never touch an existing version folder: publish uniquely named folders and
    atomically replace the `current` pointer *file*;
  - **(c)** refuse a same-version reinstall whose contents differ while the folder is
    in use.

  The ADR 0002 launcher contract reads `current` and then `<version>\climon2-cli`. So
  (b) changes what a "version folder" name means. That is a contract question for the
  launcher-freezing decision.

### 4.6 Locking in place of `flock`

- **[V] `LockFileEx`.**
  - It needs a handle with `GENERIC_READ` or `GENERIC_WRITE`.
  - An exclusive lock is mandatory for the range.
  - The lock is released at handle close or process exit, but "the time it takes …
    depends upon available system resources".
  - It is supported on ReFS
    ([LockFileEx](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-lockfileex)).
  - **[D]** Locking the home *directory* handle, as `platform.rs:63` does with `flock`,
    is not a documented use. A dedicated lock file is the documented shape.
- **[V] Rust `File::try_lock`.** It has been stable since 1.89.0. On Windows it is
  `LockFileEx(EXCLUSIVE | FAIL_IMMEDIATELY)` over the whole range, with
  `ERROR_LOCK_VIOLATION` mapped to `WouldBlock`
  (`RR library/std/src/sys/fs/windows.rs#L458-L478`; `RR library/std/src/fs.rs#L832-L865`).
  **[D]** It is portable to macOS and Linux too (flock-based there). Moving to it would
  be a shared-code change.
- **[V] Share-mode alternative.** `OpenOptionsExt::share_mode(0)` makes the open itself
  exclusive (`RR library/std/src/os/windows/fs.rs#L160-L194`).
- **[V] Handle inheritance.**
  - std files are non-inheritable by default.
  - `Command` marks only stdio inheritable
    (`RR library/std/src/sys/fs/windows.rs#L345-L351`;
    `RR library/std/src/sys/process/windows.rs#L320-L341`).
  - **[D]** So the Unix comment about a forked child holding the lock
    (`platform.rs:52-58`) has no Windows analogue unless the lock handle is passed as
    stdio.

### 4.7 Mark-of-the-Web, SmartScreen, Smart App Control

- **[V] Zone.Identifier.**
  - MOTW is the `Zone.Identifier` alternate data stream (`[ZoneTransfer] ZoneId=3`)
    ([MS-FSCC](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fscc/6e3f7352-d11c-4d76-8c39-2516a9df36e8)).
  - Browsers add it.
  - "Other methods of downloading files may not mark the files … `curl.exe`,
    `Invoke-RestMethod`, `Invoke-WebRequest`"
    (about_Execution_Policies, "Manage signed and unsigned scripts").
  - ReFS supports named streams (ReFS overview).
  - **[D]** `irm | iex` writes no file. A script-fetched zip is not guaranteed to carry
    MOTW. A browser-downloaded `.zip` will. Whether `Expand-Archive` or Explorer copies
    MOTW to extracted files is **unverified** from a primary source.
- **[V] SmartScreen.**
  - It warns on files that are not "well known and downloaded frequently" and checks
    "the digital signature used to sign a file".
  - Its edition table lists client editions only
    ([Microsoft Defender SmartScreen](https://learn.microsoft.com/en-us/windows/security/operating-system-security/virus-and-threat-protection/microsoft-defender-smartscreen/)).
- **[V] Smart App Control.** It blocks "unknown, unsigned code" by default in
  enforcement mode. When the cloud cannot decide, it allows code signed by a
  Trusted-Root-Program CA
  ([Smart App Control](https://learn.microsoft.com/en-us/windows/apps/develop/smart-app-control/overview)).
  **[D]** An unsigned, low-prevalence `install.exe` or `climon2.exe` can be blocked on a
  Windows 11 machine with SAC enforcing, independent of MOTW.
- **⚑SERVER [D]:** runners have neither SmartScreen in the client sense nor SAC, and
  Defender is disabled (§2.4). Unsigned-binary behaviour is observable only on a real
  Windows 11 machine, so it belongs to the manual acceptance checklist. Code signing is
  listed under the map's "Not yet specified".

## 5. Monitor library: `climon2_client.dll`

### 5.1 Loading by exact path

- **[V] Full paths and `LoadLibraryExW` flags.** With a fully qualified path, the
  loader "searches only that path for the module".
  - `LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR` (0x100) adds the DLL's own folder to the start of
    the dependency search, and "directories in the standard search path are not
    searched". It requires a full path.
  - `LOAD_LIBRARY_SEARCH_SYSTEM32` (0x800).
  - `LOAD_LIBRARY_SEARCH_DEFAULT_DIRS` (0x1000) = application dir, System32 and user
    dirs.
  - `LOAD_WITH_ALTERED_SEARCH_PATH` "cannot be combined with any LOAD_LIBRARY_SEARCH
    flag".
  - `LOAD_LIBRARY_REQUIRE_SIGNED_TARGET` (0x80) checks the signature at load. The page
    states no certificate policy.
  - Source: [LoadLibraryExW](https://learn.microsoft.com/en-us/windows/win32/api/libloaderapi/nf-libloaderapi-loadlibraryexw).
- **[V] Dependencies.** They are searched "as if they were loaded by using only their
  module names. That's true even if the first DLL was loaded by specifying a full
  path."
  - The standard order puts DLL redirection, API sets, SxS, the **loaded-module list**
    and KnownDLLs before any directory.
  - The package dependency graph is a step only from Windows 11 21H2 (build 22000).
    **⚑SERVER:** Server 2022 is build 20348.
  - Source: [DLL search order](https://learn.microsoft.com/en-us/windows/win32/dlls/dynamic-link-library-search-order).
- **[V] Process-wide controls.** `SetDefaultDllDirectories` is process-wide,
  irreversible, and governs dependency search for full-path loads.
  `SetDllDirectory` "affects the entire process"
  ([SetDefaultDllDirectories](https://learn.microsoft.com/en-us/windows/win32/api/libloaderapi/nf-libloaderapi-setdefaultdlldirectories);
  [DLL security](https://learn.microsoft.com/en-us/windows/win32/dlls/dynamic-link-library-security)).
- **[V] `.local` redirection.** An `app.exe.local` file or folder overrides even
  full-path loads
  ([DLL redirection](https://learn.microsoft.com/en-us/windows/win32/dlls/dynamic-link-library-redirection)).
  **[D]** This is the Windows counterpart of the Linux `LD_LIBRARY_PATH` finding: an
  ambient artifact that can redirect a release's library.
- **[D] Several versions in one process.** Two full-path loads of *different* files
  named `climon2_client.dll` should yield two modules. The docs cover "more than one
  loaded module has the same base name": a bare-name `LoadLibraryEx` returns the first,
  and bare-name `GetModuleHandleEx` is unpredictable (getmodulehandleexw).
  - A **private dependency** with the same name in two version folders can bind to
    whichever copy is already loaded, because the loaded-module list comes first. This
    mirrors the Linux soname cross-binding finding.
  - Whether `LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR` bypasses the loaded-module list is
    **undocumented**.
  - A DLL that imports only system DLLs and API sets avoids the question.
- **[D] Threat model.** Hijack and planting guidance targets attackers who control a
  search-path directory and can escalate when the victim is elevated (DLL security).
  `%LOCALAPPDATA%\climon2` is writable by every same-user process, so exact-path
  loading defends against CWD, PATH, app-dir and cross-version mix-ups, not against a
  same-user attacker.

### 5.2 Loader lock and unloading

- **[V] Loader lock.** "Threads in DllMain hold the loader lock so no additional DLLs
  can be dynamically loaded or initialized." The CRT entry point runs global
  constructors under the same restrictions
  ([DllMain](https://learn.microsoft.com/en-us/windows/win32/dlls/dllmain)).
  - Forbidden in DllMain: LoadLibrary, synchronising with other threads, waiting on
    created threads, registry, CreateProcess and more
    ([DLL best practices](https://learn.microsoft.com/en-us/windows/win32/dlls/dynamic-link-library-best-practices)).
- **[V] Rust TLS destructors.** Rust std runs thread-local destructors from a TLS
  callback in `.CRT$XLB` on thread or process detach
  (`RR library/std/src/sys/thread_local/guard/windows.rs#L71-L103`). `LocalKey` says
  "the loader lock is held while a destructor is run"
  ([LocalKey](https://doc.rust-lang.org/1.96.0/std/thread/struct.LocalKey.html)).
  **[D]** A blocking initialiser or TLS destructor in any loaded climon2 DLL stalls
  every load, unload, thread start and thread exit in the monitor. That is the
  analogue of the glibc global-lock finding.
- **[V] Unloading.**
  - On `FreeLibrary`-driven unload the DLL gets only `DLL_PROCESS_DETACH`, not
    `DLL_THREAD_DETACH` per thread (DllMain).
  - `FreeLibraryAndExitThread` exists because FreeLibrary followed by ExitThread races
    ([FreeLibraryAndExitThread](https://learn.microsoft.com/en-us/windows/win32/api/libloaderapi/nf-libloaderapi-freelibraryandexitthread)).
  - `GET_MODULE_HANDLE_EX_FLAG_PIN` keeps a module loaded until process exit
    ([GetModuleHandleExW](https://learn.microsoft.com/en-us/windows/win32/api/libloaderapi/nf-libloaderapi-getmodulehandleexw)).
  - **[D]** Other threads' Rust TLS values in the DLL are never dropped on unload.
    Any thread still inside the DLL faults after unmap.
  - No rustc or std document states that unloading a Rust cdylib is supported.
  - Options: never unload (pin), or unload only after joining every thread that
    touched the library.

### 5.3 One self-contained DLL per release (CRT)

- **[V] Default CRT linkage.** `x86_64-pc-windows-msvc` defaults to the dynamic CRT:
  `crt_static_default` is false and `crt_static_allows_dylibs` is true
  (`RR compiler/rustc_target/src/spec/base/windows_msvc.rs#L17-L33`;
  `RR compiler/rustc_target/src/spec/mod.rs#L2875`). `msvcrt.lib` pulls in
  `vcruntime` and `ucrt`
  ([CRT library features](https://learn.microsoft.com/en-us/cpp/c-runtime-library/crt-library-features?view=msvc-170)).
  - "The UCRT is now a Windows component". `vcruntime140.dll` comes from the VC++
    Redistributable or app-local deployment, which Microsoft does not recommend
    ([Redistributing Visual C++ files](https://learn.microsoft.com/en-us/cpp/windows/redistributing-visual-cpp-files?view=msvc-170)).
  - **[D]** By default every climon2 `.exe` *and* the DLL import `vcruntime140.dll`,
    which a clean Windows 11 need not have. That is not stated outright by Microsoft.
- **[V] ⚑SERVER.** Both runner images preinstall the VC++ 2022 x64 runtime (§2.4).
  **[D]** A missing-vcruntime failure is invisible in CI.
- **[V] Static CRT.** `-C target-feature=+crt-static` switches to the static CRT
  ([Rust reference, linkage](https://doc.rust-lang.org/reference/linkage.html#static-and-dynamic-c-runtimes)).
  Microsoft cautions that a DLL with a static CRT "has its own CRT state". Memory and
  CRT resources must not cross the boundary (CRT library features). **[D]** The
  planned C ABI already passes by value or into caller memory.
- **[V] Rust std imports.** std 1.96 links `kernel32`, `ntdll`, `ws2_32`, `userenv`
  and `dbghelp`, plus raw-dylib `bcryptprimitives` and `api-ms-win-core-synch-l1-2-0`
  (`RR library/windows_link/src/lib.rs#L45-L52`;
  `RR library/std/src/sys/pal/windows/c.rs#L101-L243`). All are System32, KnownDLL or
  API-set names. **[D]** The real import table needs `dumpbin /dependents` on a CI
  build.
- **[D] The `crt-static` trap from §2.1.** It applies here too: setting `crt-static`
  in Cargo config is overridden by the coverage `RUSTFLAGS`.

### 5.4 Notification primitive in place of `eventfd`

- **[V] Event objects.**
  - `WaitForMultipleObjects` handles at most `MAXIMUM_WAIT_OBJECTS`.
  - Closing a handle while a wait is pending gives undefined behaviour
    ([WaitForMultipleObjects](https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-waitformultipleobjects)).
  - Thread-pool waits are one-shot, and require `CloseThreadpoolWait` before closing
    the handle
    ([SetThreadpoolWait](https://learn.microsoft.com/en-us/windows/win32/api/threadpoolapiset/nf-threadpoolapiset-setthreadpoolwait)).
  - `DuplicateHandle` gives the host its own reference
    ([DuplicateHandle](https://learn.microsoft.com/en-us/windows/win32/api/handleapi/nf-handleapi-duplicatehandle)).
  - **[D]** The Linux "borrowed until destroy" rule carries over as a hard requirement.
- **[V] Sockets.**
  - AF_UNIX on Windows has no `socketpair`, no datagram or seqpacket, and no ancillary
    data
    ([AF_UNIX comes to Windows, Microsoft devblog](https://devblogs.microsoft.com/commandline/af_unix-comes-to-windows/)).
  - `WSAPoll` reports failed connects correctly from Windows 10 2004
    ([WSAPoll](https://learn.microsoft.com/en-us/windows/win32/api/winsock2/nf-winsock2-wsapoll)).
  - `WSAEventSelect` maps socket readiness to an event, forces non-blocking mode, and
    allows one association per socket
    ([WSAEventSelect](https://learn.microsoft.com/en-us/windows/win32/api/winsock2/nf-winsock2-wsaeventselect)).
  - A `SOCKET` is not a file descriptor
    ([SOCKET data type](https://learn.microsoft.com/en-us/windows/win32/winsock/socket-data-type-2)).
- **[V] IOCP.** A port is not sharable between processes, and duplicating one gives an
  unusable handle. `PostQueuedCompletionStatus` can signal custom events
  ([I/O completion ports](https://learn.microsoft.com/en-us/windows/win32/fileio/i-o-completion-ports)).
- **[V] mio 1.2.5.**
  - It polls sockets via IOCP and `\Device\Afd`, and "currently only support[s]
    socket".
  - Its `Waker` is a `PostQueuedCompletionStatus`
    ([mio `selector.rs`, `afd.rs`, `waker.rs` @ v1.2.5](https://github.com/tokio-rs/mio/tree/v1.2.5/src/sys/windows)).
  - **[D]** A socket descriptor integrates with mio directly. An event `HANDLE` needs a
    bridge thread or thread-pool wait. Both types are pointer-sized, which matters for
    the C ABI.
- **[V] Returning a 48-byte struct by value.** The x64 convention returns it via a
  caller-allocated buffer, with the hidden pointer as the first argument (RCX), and
  returns that pointer in RAX
  ([x64 calling convention](https://learn.microsoft.com/en-us/cpp/build/x64-calling-convention?view=msvc-170#return-values)).
  `extern "C"` is the x64 Windows convention
  ([Rust reference, external blocks](https://doc.rust-lang.org/1.96.0/reference/items/external-blocks.html)).
  `repr(C)` uses MSVC's aggregate layout algorithm, and C `long` is 4 bytes
  ([x64 software conventions](https://learn.microsoft.com/en-us/cpp/build/x64-software-conventions?view=msvc-170#x64-aggregate-and-union-layout)).
  **[D]** The spec's layout asserts need a Windows run.

## 6. Options surfaced for downstream decisions (no choice made)

1. **Coverage on Windows.** Add an `xtask` arm with `-runtime-counter-relocation`.
   Prove `%c` survival under `TerminateProcess` on the runner, or accept
   exit-flush-only coverage for terminated processes.
2. **Mutator portability.** Changing the pinned mutator (killpg, cmd-shell
   `--test-command`) is out-of-repo. In-repo options:
   - pass an argv-safe test command, e.g. a cmd-compatible string or a `.cmd` wrapper;
   - accept that a mutant timeout crashes the run;
   - get an upstream fix and re-pin.
3. **Workers and caches.** A Windows seed for `test-mutation.py` without `cp`, or cold
   builds.
4. **Runner pinning.** Pin `windows-2025` (Server 2025, build 26100) rather than
   `windows-latest`. The map's Windows 11 baseline is never what CI runs; ⚑SERVER
   gaps go to the manual checklist.
5. **Archive.** Build the zip with bsdtar, a Rust crate or PowerShell. Produce the
   checksum lines in `xtask`. Third-artifact assembly goes in `publish`.
6. **Launcher replacement.** The rename-aside premise is unconfirmed by documentation
   and needs a CI probe first. Choose between hard link and copy. Choose a
   deferred-cleanup point.
7. **Version folders.** Replacing a folder in use is impossible, so choose unique
   folders plus a pointer, or refuse.
8. **Install lock.** Use a lock file (`File::try_lock` or share mode 0) instead of a
   directory lock.
9. **CRT.** Use the dynamic CRT plus a vcruntime requirement or bundling, or
   `+crt-static`, set so that coverage builds keep it.
10. **Library loading.**
    - Flags: `LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | SYSTEM32`, or SYSTEM32 only for a
      system-imports-only DLL.
    - Unload policy: pin or join.
    - Notification type: event `HANDLE` or `SOCKET`.

## 7. Open / unverified (needs a Windows CI experiment or a better source)

1. Renaming an image-mapped `.exe` (and its hard-link sibling) aside: no primary
   statement; the documentation literally contradicts it.
2. `%c` continuous coverage under COFF and `TerminateProcess`: runtime code exists,
   docs say untested.
3. Exact imports of a 1.96 cdylib and `.exe` (`dumpbin /dependents`), and whether a
   clean Windows 11 lacks `vcruntime140.dll`.
4. Whether `LOAD_LIBRARY_SEARCH_*` skips the loaded-module list for dependencies, and
   whether two same-named DLLs at different paths load as two modules.
5. Safety of `FreeLibrary` on a Rust cdylib.
6. MOTW propagation by `Expand-Archive` and Explorer; the SmartScreen trigger path;
   Smart App Control availability conditions.
7. The .NET Framework target of `powershell.exe` 5.1 (TLS default); whether `irm` gets
   the KB5074596 prompt; .NET Framework `SetEnvironmentVariable` behaviour.
8. ReFS support for hard links and POSIX rename/delete (conflicting Microsoft tables).
9. Wall-clock of a Windows `--native-only` gate: no measurement exists.
10. Whether `python3` resolves on the runner after `actions/setup-python` (the
    `CLIMON2_QUALITY_PYTHON` override exists).
