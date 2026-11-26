# Install

The supported platforms, and every way to install climon2.

## Platforms

climon2 runs on:

- **macOS 15 or newer**, on Apple Silicon and Intel. The macOS programs are
  universal and ad-hoc signed. They are not signed with a Developer ID or
  notarised.
- **x86_64 GNU/Linux** with glibc 2.39 or newer, tested on Ubuntu 24.04.
- **x86_64 Windows 11**, build 26100 or newer. See [Windows](#windows).

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
A climon2 home belongs to one computer, and on Linux to one PID namespace.
climon2 refuses a home that another machine or container uses; see
[one climon2 home, one computer](commands.md#one-climon2-home-one-computer).

## Windows

Windows needs x86_64 Windows 11, build 26100 or newer. The build is documented,
not checked when climon2 runs; Windows 10 is not supported. Nothing else needs
installing first: every climon2 program has Microsoft's C runtime built in.

In Windows PowerShell 5.1, which Windows 11 includes, or PowerShell 7, install
the latest release with:

```powershell
irm https://github.com/jackgeek/climon2/releases/latest/download/install.ps1 | iex
```

Pinned release (replace `0.1.0` with the published version):

```powershell
irm https://github.com/jackgeek/climon2/releases/download/v0.1.0/install.ps1 | iex
```

Each published `install.ps1` downloads only its stamped version's archive,
`climon2-<version>-windows-x86_64-msvc.zip`, checks its SHA-256 against the
release's `SHA256SUMS`, and runs the `install.exe` inside it. Piping it to
`iex` works under Windows' default execution policy. It never closes your
window: a failure is a PowerShell error starting `climon2-install.ps1: error:`.
To pass options, run it as a script block:

```powershell
& ([scriptblock]::Create((irm https://github.com/jackgeek/climon2/releases/latest/download/install.ps1))) -Dir "$env:USERPROFILE\tools\climon2" -NoModifyPath
```

The default executable home is `%LOCALAPPDATA%\climon2`, and `-Dir` chooses
another. Unlike the macOS and Linux installers, the Windows installer adds the
executable home to your user `PATH`, at the end so `c2` never takes precedence
over another tool, without administrator rights. It prints a line when it does.
Terminals that are already open do not see the change; open a new one. A later
install into another directory replaces the entry the installer added, so there
is never more than one. `-NoModifyPath` leaves `PATH` alone and prints the
entry to add by hand.

Upgrades keep older version folders, and live sessions keep running on their
own version. If a `c2` command is running during an upgrade, the installed
`c2.exe` launcher is kept, which is safe because every launcher runs every
version, and the next install made while none is running replaces it.

To install from a browser download instead, unblock the zip before extracting
it, check it against `SHA256SUMS` and run its installer. `install.exe` takes
`--dir <directory>` and `--no-modify-path`:

```powershell
Unblock-File climon2-0.1.0-windows-x86_64-msvc.zip
(Get-FileHash -Algorithm SHA256 climon2-0.1.0-windows-x86_64-msvc.zip).Hash   # compare with SHA256SUMS
Expand-Archive climon2-0.1.0-windows-x86_64-msvc.zip -DestinationPath .
.\climon2-0.1.0-windows-x86_64-msvc\install.exe
```

The climon2 home is `%USERPROFILE%\.climon2` by default; `CLIMON2_HOME`
overrides it, as on the other platforms.
