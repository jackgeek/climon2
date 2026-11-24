# Install

The supported platforms, and every way to install climon2.

## Platforms

climon2 runs on:

- **macOS 15 or newer**, on Apple Silicon and Intel. The macOS programs are
  universal and ad-hoc signed. They are not signed with a Developer ID or
  notarised.
- **x86_64 GNU/Linux** with glibc 2.39 or newer, tested on Ubuntu 24.04.

Arch Linux is supported for development. musl and Linux arm64 are not tested.
See [distribution](../distribution.md) for the archives for each
platform.

## Install script

Latest release:

```sh
curl -fsSL https://github.com/jackgeek/climon2/releases/latest/download/install.sh | /bin/bash
```

A specific release (replace `0.1.0` with the version you want):

```sh
curl -fsSL https://github.com/jackgeek/climon2/releases/download/v0.1.0/install.sh | /bin/bash
```

Each script downloads only its own version and checks its SHA-256 checksum
before it runs the installer.

The programs go in the **executable home**, `~/.local/share/climon2` by
default. To pick another folder:

```sh
curl -fsSL https://github.com/jackgeek/climon2/releases/latest/download/install.sh |
  /bin/bash -s -- --dir "$HOME/tools/climon2"
```

Add the executable home to your PATH:

```sh
export PATH="$HOME/.local/share/climon2:$PATH"
```

For a custom folder, the installer prints the matching line. It never edits
your shell start-up files. Upgrades keep the existing version folders and
running sessions. Every install replaces `c2`, the launcher, in the executable
home. You do not need `sudo` or Homebrew.

## Installing from an archive

If you download on macOS with a browser, check `SHA256SUMS`, unpack the
archive, then remove the quarantine flag from the unpacked folder before you
run its installer:

```sh
shasum -a 256 -c SHA256SUMS
tar -xzf climon2-0.1.0-macos-universal.tar.gz
xattr -dr com.apple.quarantine climon2-0.1.0-macos-universal
./climon2-0.1.0-macos-universal/install
```

To install by hand on Linux, use the `linux-x86_64-gnu` archive and
`sha256sum`. There is no quarantine step. Linux needs procfs, and a file
system at the install path that can swap two folders with `renameat2`.
Processes that share a climon2 home must share the same PID namespace. Sharing
session data across containers or PID namespaces is not supported.
