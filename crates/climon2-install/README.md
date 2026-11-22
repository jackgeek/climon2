# climon2-install

The installer. On macOS and GNU/Linux it ships as `install`, on Windows as
`install.exe` ([Windows](#windows) below); `src/lib.rs` picks `src/unix.rs` or
`src/windows.rs` by a `cfg` on that one module declaration.

The macOS and GNU/Linux installer is shipped as `install` beside the launcher `c2` and the release's
`<version>/{climon2-cli,climon2-daemon,climon2-client,climon2-monitor}` executables
and client library (`libclimon2_client.dylib` on macOS, `libclimon2_client.so`
on Linux). The client library is published atomically with the rest of its
version.

`install [--dir <directory>]` defaults to `$HOME/.local/share/climon2`.
It prints a shell-quoted `export PATH=…` hint only when the canonical destination
is absent from PATH. It installs the launcher as `c2`, the only command name (#303); `LAUNCHER`
is that file name.
It never changes shell profiles or signing/quarantine attributes. Errors produce one
`climon2: error: …` line on stderr and status 125.

## Library boundary

- `Payload::from_directory(directory, version)` validates a release payload.
- `Payload::beside_installer()` resolves the running installer's canonical path
  and uses the compiled package version.
- `Payload::from_siblings(directory, version)` reads `c2`, `climon2-cli`,
  `climon2-daemon`, `climon2-client` and `climon2-monitor` directly from one
  Cargo output directory, and the client library from its `deps/`
  subdirectory. Packaged tests can therefore install the exact instrumented
  sibling artifacts, without nested Cargo invocations or uninstrumented
  substitutes.
- `version_files()` names the files of a version folder, in installation
  order: the executables, then the client library, whose name comes from
  `climon2_core::layout::client_library_name`. `cargo_output(directory, name)`
  is where a Cargo build leaves each of them: Cargo uplifts binaries but leaves
  a dependency's cdylib in `deps/`, beside the test executables. xtask's
  release check, tooling probe and install-script test use both.
- `Payload::version()` returns the validated core `VersionName`.
- `install(&payload, destination)` returns `InstallReport { executable_home }`,
  with a canonical destination. It does not print or inspect environment.
- `run(args)` is the command adapter; arguments exclude argv[0].

Payload files, including the client library, must be regular executable
files, not symlinks. Installation copies rather than mutates them. Identical
versions retain their existing files;
different same-version contents replace the whole version atomically using
Darwin `renamex_np(RENAME_SWAP)` or Linux `renameat2(RENAME_EXCHANGE)`.
Linux requires a filesystem supporting atomic directory exchange; an unsupported
filesystem fails installation rather than falling back to non-atomic clobbering.
The syscall adapters live in `src/platform/{macos,linux}.rs`, separately
registered for native quality analysis; argument validation and publication
remain shared.
The launcher is always replaced atomically,
including on an identical reinstall, per #21.19. Other versions are retained.
The atomic `current` replacement is the final publication.

Nothing is migrated. A `c2 -> climon2` symlink from an install before #303 is
replaced by the launcher like any other `c2`; the `climon2` it named is left in
place.

## Interruption and concurrency

An exclusive nonblocking lock on the executable-home directory serializes
installers. `.climon2-install-stage` is reserved for this installer's staging and
recovery; no prefix scan or cleanup of other directories occurs. Returned errors
clean staging through its owner. Process interruption can leave staging behind;
the next installer removes it under the same lock before restaging. A published
version is always complete, even when `current` still names the previous one.
This is process-interruption recovery, not a power-loss durability guarantee.

Only the compile-time `test-hooks` feature enables
`CLIMON2_TEST_INSTALL_FAULT=fail:<boundary>` or `crash:<boundary>`.
Boundaries are `copy-<file>` after each version file is staged (for example
`copy-climon2-cli`), `version-staged`, `version-published`, and
`launcher-published`; crash exits 86 without Rust cleanup. Production builds do
not contain these hooks.

`tests/install.rs` documents the #21/#24/#33 rules at the public library and
real-installer boundaries. The #33 concurrent-fork lock test lives in
`tests/lock.rs`, a separate test process, because its forked children inherit
every descriptor of their process, including pipes other tests close. Run both
with the sibling launcher built in the same Cargo target:

```sh
cargo test -p climon2-launcher -p climon2-install --features climon2-install/test-hooks
```

## Windows

`install.exe [--dir <directory>] [--no-modify-path]` installs the release
beside it into `%LOCALAPPDATA%\climon2` or the `--dir` home, as
[Windows packaging and upgrades](../../docs/windows-distribution.md) specifies.
Its code is in two registered Windows files, which the Windows quality gate
holds to CRAP 5 and zero surviving mutants: `src/windows.rs` (options, the
lock, staging, the version folder, the launcher and `current`) and
`src/user_path/windows.rs` (the user `PATH` and its record in the registry).
The library exports `run`, `LAUNCHER` (`c2.exe`) and `version_files()`, the
version folder's files, which xtask's Windows dist tests against its zip.

- `install.lock` in the home is created and locked exclusively, failing at
  once if another installer holds it; it is never renamed or deleted.
- The version folder is copied into `.climon2-install-stage` and renamed into
  place. An identical installed folder is kept. A different one is renamed
  aside, replaced and deleted, unless one of its `.exe` files cannot be
  opened for writing, which Windows refuses for a running program: then the
  install fails with 125 and nothing changes.
- `c2.exe` is replaced unless identical; one Windows will not replace, such as
  a running one, is kept, with one line on standard output saying so.
- `current` is written last, by renaming a staged file over it.
- Unless `--no-modify-path`, the home is appended to the stored
  `HKCU\Environment\Path` as one literal entry, keeping the value's registry
  type, and recorded as `PathEntry` under `HKCU\Software\climon2`. A later
  install into another home removes the recorded entry. An entry that already
  names the home, after expanding variables and ignoring case and a trailing
  backslash, counts as present and is not recorded. A change is broadcast
  with `WM_SETTINGCHANGE`. `--no-modify-path` touches neither value and prints
  the entry to add by hand.

`tests/windows.rs` runs this test binary itself as `install.exe`, after
redirecting `HKEY_CURRENT_USER` to a registry key of its own with
`RegOverridePredefKey`, so no test, and no mutant under test, can change the
real user `PATH`. Each install also gets its own `LOCALAPPDATA` and working
directory. `tests/archive_windows.rs` is the Windows Archive job's: it tests
the zip named by `CLIMON2_E2E_ARCHIVE` (its `install.exe`, and `install.ps1`
under both PowerShells) and passes without running when that is unset. Its
tests that change the real user `PATH` also need
`CLIMON2_E2E_REAL_USER_PATH=1`, which only the Archive job's throwaway runner
sets.

```powershell
cargo test -p climon2-install
```