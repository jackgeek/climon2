# climon2-install

The macOS installer is shipped as `install` beside `climon2` and the release's
`<version>/{climon2-cli,climon2-daemon,climon2-client}` directory.

`install [--dir <directory>]` defaults to `$HOME/.local/share/climon2`.
It prints a shell-quoted `export PATH=…` hint only when the canonical destination
is absent from PATH. It never changes shell profiles, creates symlinks, or
changes signing/quarantine attributes. Errors produce one
`climon2: error: …` line on stderr and status 125.

## Library boundary

- `Payload::from_directory(directory, version)` validates a release payload.
- `Payload::beside_installer()` resolves the running installer's canonical path
  and uses the compiled package version.
- `Payload::from_siblings(directory, version)` reads `climon2`, `climon2-cli`,
  `climon2-daemon` and `climon2-client` directly from one Cargo output directory.
  Packaged tests can therefore install the exact instrumented sibling binaries,
  without nested Cargo invocations or uninstrumented substitutes.
- `Payload::version()` returns the validated core `VersionName`.
- `install(&payload, destination)` returns `InstallReport { executable_home }`,
  with a canonical destination. It does not print or inspect environment.
- `run(args)` is the command adapter; arguments exclude argv[0].

Payload files must be regular executable files, not symlinks. Installation
copies rather than mutates them. Identical versions retain their existing files;
different same-version contents replace the whole version atomically using
Darwin `renamex_np(RENAME_SWAP)`. The launcher is always replaced atomically,
including on an identical reinstall, per #21.19. Other versions are retained.
The atomic `current` replacement is the final publication.

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
Boundaries are `copy-cli`, `version-staged`, `version-published`, and
`launcher-published`; crash exits 86 without Rust cleanup. Production builds do
not contain these hooks.

`tests/install.rs` documents the #21/#24/#33 rules at the public library and
real-installer boundaries. Run with the sibling launcher built in the same
Cargo target:

```sh
cargo test -p climon2-launcher -p climon2-install --features climon2-install/test-hooks
```
