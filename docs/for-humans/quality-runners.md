# Quality runners on the owner's hardware

This document says how climon2's CRAP and mutation gates run on the owner's
own machines instead of GitHub's, and how to set each machine up. It is for
contributors, and mainly for the owner, who keeps the machines. The workflow
is `.github/workflows/quality.yml`.

**Audience:** Contributors.

## What runs where

Each platform's gate runs on its own machine, so the three run at once.

| Machine | Runner labels | Gate | What it qualifies |
| --- | --- | --- | --- |
| Omarchy PC (Arch Linux) | `self-hosted`, `Linux`, `X64`, `climon2-quality` | `cargo xtask mutate` | Shared source and Linux-only source |
| Windows 11 VM on that PC | `self-hosted`, `Windows`, `X64`, `climon2-quality` | `cargo xtask mutate --native-only` | Registered Windows files |
| 2019 MacBook (Intel) | `self-hosted`, `macOS`, `X64`, `climon2-quality` | `cargo xtask mutate --native-only` | macOS-only source |

Every gate mutates only the source a push changes. On `main` that means the
push itself, and on any other branch it means the branch's difference from
`main`. A gate with nothing to mutate passes in a minute.

Release qualification does not use these machines. It still runs on
GitHub-hosted runners, from `check.yml`, so its evidence records name a
known image.

The Mac is an Intel machine, while GitHub's macOS gate runs on Apple silicon.
macOS-only source that differs by architecture is qualified here on x86-64
only.

## Why the jobs run on push

The repository is public. A pull request from a fork runs the workflow file
from the fork, so any guard written in the workflow could be deleted by the
pull request it guards. These jobs therefore never run on `pull_request`.
They run on `push`, which only people with write access can do. GitHub shows
each result on the pull request whose head is that commit, as with any other
check.

The machines still run code from any branch pushed to the repository. Each
runner runs as its own account, so a job cannot read the owner's files or
credentials. On the Windows VM that account must be an administrator: the
gate's tests expect a full administrator token, as the GitHub runner has. An
administrator can read anything on the VM, so the agents' GitHub credentials
there are not protected from a job. Only push access protects them.

## Turning it on

A job runs only while the repository variable `CLIMON2_OWN_RUNNERS` lists its
platform, for example `linux,windows,macos`. A push therefore never waits on
a machine that is not set up yet. Set it under Settings > Secrets and
variables > Actions > Variables, and add each platform as its machine comes
online.

Once a platform is listed, its gate's result is the check to wait for. The
local gate in [Running the gate locally](windows/quality.md#running-the-gate-locally)
is no longer needed for that platform.

## Setting up a machine

Every machine needs the same base:

1. **A runner account.** Make a separate account for the runner, such as
   `c2runner`, rather than using your own.
2. **The tools.** Install Git, [rustup](https://rustup.rs) (the repository's
   `rust-toolchain.toml` picks the toolchain) and Python 3.11 or later.
3. **The runner.** Under Settings > Actions > Runners > New self-hosted
   runner, pick the machine's OS, then follow the download steps as the
   runner account. Configure it with the extra label and a name:

   ```sh
   ./config.sh --url https://github.com/jackgeek/climon2 --token <token> \
     --name <machine> --labels climon2-quality --unattended
   ```

4. **A service.** Install the runner as a service so it starts at boot:
   `sudo ./svc.sh install c2runner` and `sudo ./svc.sh start` on Linux. On
   Windows, answer yes to "run as service" during `config.cmd` and give it the
   runner account. macOS needs a different service, described under
   [MacBook](#macbook).
5. **Its environment.** Put machine-specific settings in the `.env` file in
   the runner's folder, one `NAME=value` per line, then restart the service.

Each gate keeps its build and its quality tools in the runner's work folder
between runs (`clean: false`), so only the first run builds everything.

### Omarchy PC

Install the test prerequisites the Linux CI job uses:

```sh
sudo pacman -S --needed base-devel git rustup python clang shellcheck vim less
```

`/bin/bash` must be GNU bash 5, which it is on Arch. The Linux gate always
uses 8 workers.

### Windows VM

Install Python 3.13, Git for Windows (the jobs run in its bash) and the
Visual Studio Build Tools with the C++ workload. Make the runner account a
local administrator.

Create, once, the second, non-administrator account the cross-account test
logs on as. In an elevated PowerShell:

```powershell
$password = Read-Host -AsSecureString
New-LocalUser -Name c2second -Password $password -AccountNeverExpires -PasswordNeverExpires
Add-LocalGroupMember -Group Users -Member c2second
```

Then the runner's `.env`:

```text
CLIMON2_QUALITY_PYTHON=C:\Program Files\Python313\python.exe
CLIMON2_NATIVE_MUTATION_WORKERS=4
CLIMON2_TEST_CONPTY_CACHE=C:\c2runner\conpty
CLIMON2_SECOND_ACCOUNT=c2second
CLIMON2_SECOND_ACCOUNT_PASSWORD=<its password>
```

Use the Python path the installer reports.

### MacBook

Install the Xcode Command Line Tools (`xcode-select --install`), then
[Homebrew](https://brew.sh), and through it `python@3.13`, `shellcheck` and
`bash`. The job selects Homebrew's bash 5, never `/bin/bash` 3.2. Homebrew
installs Python as `python3.13` only, so `python3` on the path is still
Apple's 3.9, which is too old; the `.env` names the right one. In the
runner's `.env`:

```text
CLIMON2_NATIVE_MUTATION_WORKERS=4
CLIMON2_QUALITY_PYTHON=/usr/local/bin/python3.13
```

The runner also reads its path from `.path` in its folder. Make that one line
that puts the runner account's `~/.cargo/bin` and `/usr/local/bin` first:

```text
/Users/c2runner/.cargo/bin:/usr/local/bin:/usr/local/sbin:/usr/bin:/bin:/usr/sbin:/sbin
```

Do not use `svc.sh` on macOS. It installs a launch agent, which runs only
while its account is logged in, and a runner account never logs in. Install
a launch daemon instead, which starts at boot and runs as the runner account:

```sh
cd /Users/c2runner/actions-runner
label=actions.runner.jackgeek-climon2.<machine>
sudo cp bin/runsvc.sh runsvc.sh && sudo chown c2runner:staff runsvc.sh
sudo -u c2runner mkdir -p /Users/c2runner/Library/Logs/$label
sed -e "s|{{SvcName}}|$label|g" -e "s|{{RunnerRoot}}|$PWD|g" \
    -e "s|{{User}}|c2runner|g" -e "s|{{UserHome}}|/Users/c2runner|g" \
    bin/actions.runner.plist.template | sudo tee /Library/LaunchDaemons/$label.plist >/dev/null
sudo launchctl bootstrap system /Library/LaunchDaemons/$label.plist
```

To restart it after changing `.env`, run
`sudo launchctl kickstart -k system/$label`.

Stop the Mac sleeping while it is plugged in (System Settings > Battery >
Options, or `sudo pmset -c sleep 0`), or a job waits until it wakes.

## Sharing the PC

The Linux gate and the Windows gate run on the same PC, the Windows one
inside the VM, and they run at the same time. Size the VM and the worker caps
so that both fit in the PC's memory without swapping. Each mutation worker
builds the workspace on its own. On the VM (16 cores, 16 GB) 16 workers did
not fit, and 2 ran beside other builds. Start the VM at 4 and raise it while
the PC has memory to spare.
