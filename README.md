# climon2

Run any terminal command in a managed session, or list live sessions:

```sh
climon2 exec bash --noprofile --norc
climon2 exec vim
climon2 list
climon2 list --json
```

The command keeps its arguments, working directory and environment. Its exit
status becomes climon2's exit status. Leaving the client ends the session; detach,
reattach and screen restoration are not supported. Nested sessions are supported.
Session data defaults to `~/.climon2`; set `CLIMON2_HOME` to override it.

## Install

Requires **macOS 15 or newer**, Apple Silicon or Intel. Release binaries are
universal and ad-hoc signed, without Developer ID signing or notarisation.

Latest release:

```sh
curl -fsSL https://github.com/jackgeek/climon2/releases/latest/download/install.sh | /bin/bash
```

Pinned release (replace `0.1.0` with the published version):

```sh
curl -fsSL https://github.com/jackgeek/climon2/releases/download/v0.1.0/install.sh | /bin/bash
```

Each published script downloads only its stamped version and checks its SHA-256
checksum before running the installer. To choose another executable home:

```sh
curl -fsSL https://github.com/jackgeek/climon2/releases/latest/download/install.sh |
  /bin/bash -s -- --dir "$HOME/tools/climon2"
```

The default executable home is `~/.local/share/climon2`. Add it to PATH:

```sh
export PATH="$HOME/.local/share/climon2:$PATH"
```

The installer prints a matching hint for custom destinations and never modifies
shell startup files. Upgrades preserve existing version folders and live sessions.
There is no `sudo` or Homebrew installation step.

For browser downloads, verify `SHA256SUMS`, extract the archive, then remove
quarantine from the extracted directory before running its installer:

```sh
shasum -a 256 -c SHA256SUMS
tar -xzf climon2-0.1.0-macos-universal.tar.gz
xattr -dr com.apple.quarantine climon2-0.1.0-macos-universal
./climon2-0.1.0-macos-universal/install
```

## Diagnostics

Operational failures use status 125 and identify a log file. Logs contain climon2 diagnostics,
not terminal content, command arguments or environment values. Release logging
defaults to INFO; `CLIMON2_LOG` accepts a tracing filter. Session logs are capped
at 8 MiB each; inactive session logs older than seven days are pruned.

## Development and releases

```sh
cargo test --workspace --locked
cargo xtask setup-quality
cargo xtask check
cargo xtask check --mutate-all
cargo xtask dist
```

Install ShellCheck before `xtask check`. Full tests need controlled bash 3.2 and
5.x, vim and less. `CLIMON2_TEST_BASH` selects one required shell; missing shells
fail in CI. Linux and Windows run only neutral contracts (Linux also tests the
Unix launcher), not the macOS product.

`xtask check` runs formatting, strict clippy, unit/integration/installer-built
packaged tests, ShellCheck, append-only metadata fixtures, instrumentation and
worker-isolation proofs, coverage-backed CRAP <= 5 and zero-survivor mutation.
`CLIMON2_MUTATION_BASE` selects the changed-source comparison (default `HEAD`).

`xtask dist` requires an **arm64 macOS host** and both Rust macOS targets. It
builds isolated, unhooked packages with deployment target 15.0, merges and
ad-hoc signs each executable, and checks signatures, architectures and versions.
It writes the universal archive, version-stamped `install.sh` and `SHA256SUMS`
to `target/dist/`. Set `CLIMON2_E2E_ARCHIVE` to that archive to test it unchanged.

A `v<workspace version>` tag runs full mutation and all gates. The same final
archive must pass both architectures and bash profiles, including vim and less,
before a non-draft GitHub Release and provenance attestation are published.
`-rc.N` tags are prereleases. Starting with the second published release, actual
previous-release compatibility is mandatory; the first release invents none.

**Intel runner availability is a release blocker.** If `macos-15-intel` retires,
a replacement native Intel macOS 15+ runner must pass the same artifact tests.
An unavailable runner or arm64-only success does not establish Intel support.

MIT licensed; see [LICENSE](LICENSE).
