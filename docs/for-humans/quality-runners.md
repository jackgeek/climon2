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

`/bin/bash` must be GNU bash 5, which it is on Arch. The workflow runs the
Linux gate with four mutation workers (`CLIMON2_MUTATION_WORKERS`); see
[Sharing the PC](#sharing-the-pc).

### Windows VM

Install Python 3.13, Git for Windows (the jobs run in its bash), the Visual
Studio Build Tools with the C++ workload, and PowerShell 7 at
`C:\Program Files\PowerShell\7`. The tests use that PowerShell 7 and
refuse the Microsoft Store copy. Make the runner account a local
administrator.

If an installer cannot run, two copies work instead. The runner on the
owner's VM uses both:

- **Python:** a copy of another Python 3.13 installation's folder, such as
  `C:\c2runner\python313`.
- **PowerShell 7:** the release's `win-x64` zip unpacked into
  `C:\Program Files\PowerShell\7`, once its SHA-256 matches the release's.

Give the runner its own Rust as well. Set `RUSTUP_HOME` and `CARGO_HOME` to
folders of its own, and put their `bin` folder first on the `PATH` in its
`.env`. Otherwise it would use the toolchain of whoever installed rustup.

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
CLIMON2_TEST_CONPTY_CACHE=C:\c2runner\conpty
CLIMON2_SECOND_ACCOUNT=c2second
CLIMON2_SECOND_ACCOUNT_PASSWORD=<its password>
```

Use the Python path the installer reports.

### MacBook

Install the Xcode Command Line Tools (`xcode-select --install`), then
[Homebrew](https://brew.sh), and through it `python@3.13`, `shellcheck` and
`bash`. The job uses the bash 5 in `/opt/homebrew/bin` or `/usr/local/bin`,
never `/bin/bash` 3.2, and asks Homebrew only when neither is bash 5. The
runner account need not own Homebrew. Homebrew installs Python as
`python3.13` only, so `python3` on the path is still Apple's 3.9, which is
too old; the `.env` names the right one. In the runner's `.env`:

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

## Timeouts

A hung test once held the Linux runner for six hours while every other push
waited ([#401](https://github.com/jackgeek/climon2/issues/401)). Two kinds of
timeout now bound a gate.

The first bounds each mutant. A mutant's tests run in **stages**: its library
tests, then its crate's integration tests, then the whole suite, which is the
full workspace except on Windows. Before any mutant runs,
`tools/test-mutation.py` times each stage once, unloaded. Each earlier stage
then gets 10 times its run, and the mutant's whole run gets 10 times one run
of the suite, which keeps it inside the mutator's own timeout. A mutant that
outlives a timeout counts as killed, as one that fails a test does.

When a stage ends, whether it passed, failed or timed out, the script ends
every process its tests started. That includes a `climon2-daemon` that left
the test's session and whose parent has exited, which killing a process group
misses. The hung runs left exactly such daemons, still holding the mutator's
output pipe, so the mutator waited for them. The script finds these processes
by parent and by an executable in the worker's build folder or in the
private temporary folder (`TMPDIR`, or `TMP` and `TEMP`) it gives the tests.
On Linux it also adopts orphaned processes, so parents alone find them. The
tests write to a file rather than to the mutator's pipe, so a process the
search missed could not stall the mutator either.

The second bounds each job. `quality.yml` gives each a `timeout-minutes`, a
clear margin over the longest passing run of its gate. GitHub fails a job
that reaches it, ends the job's processes, and the runner takes the next job.

| Gate | Longest passing run, by 2026-11-25 | Timeout |
| --- | --- | --- |
| Linux | 171 min, 287 mutants; the others 4-109 min | 240 min |
| Windows | 68 min | 120 min |
| macOS | none yet | 120 min |

## Sharing the PC

The Linux gate and the Windows gate run on the same PC, the Windows one
inside the VM, and they run at the same time. So the workflow limits their
mutation workers, set in `quality.yml`: four on Linux, two on Windows.

How many is a trade-off. With 6 workers each, with 2 build jobs and 2 test
threads per worker, end-to-end tests missed their deadlines on both gates.
With one worker each, nothing failed, but the PC sat at 1-14% busy: a worker
spends most of its time waiting on its tests' processes, pipes and timeouts.
So on 2026-11-23 the owner asked to raise the workers until tests start to
fail, then back off a little. Three workers passed on both gates at once
on 2026-11-23, finishing the Linux gate in 1 h 50 min and the Windows gate
in 1 h 8 min. But the Windows VM's 16 GB ran critically low on memory while
they did, so only the slower Linux gate stepped up to four. Then, with Linux
at four, the Windows gate's load still made the Linux gate's coverage-stage
timing tests fail twice running (an 8 MiB flood with a 15 s deadline took
29 s), so Windows went down to two.

Each worker builds and tests the workspace on its own.
`CLIMON2_MUTATION_WORKER_JOBS`, also set in `quality.yml`, gives its Cargo 4
build jobs and 4 test threads. The gate measures its stage timeouts under
the same limit.

The rest of the gate, such as its own coverage build and test run, gets 4
build jobs and 4 test threads (`CARGO_BUILD_JOBS` and `RUST_TEST_THREADS`).
Without a limit, one gate's baseline test run used every thread its machine
has, and the other gate's timing tests failed while it ran. At 8, the Linux
gate's coverage-stage timing tests still failed while the Windows gate built
and tested at the same time, so on 2026-11-23 the owner chose 4.

Each worker also has its own target folder. On Windows, one with debug info
nears 8.4 GB, and 6 of them filled the VM's disk. So every gate builds its
workers without debug info, which halves that.

Change the values in `quality.yml` to resize either.

## The Mac's test threads

The Mac's gate runs its tests 4 at a time as well
([#425](https://github.com/jackgeek/climon2/issues/425)). Before that it ran
16 at a time on the laptop's 8 cores, and the same 8 MiB flood missed its
15 s deadline in 4 of 5 gates, at 15.3 to 16.4 s. Alone, the flood takes
about 3 s, or 4 s in the gate's coverage build. It slows as other work takes the CPU, and under load the laptop
throttles itself to 58% of its speed. With 16 other busy processes running,
the flood took 11.3 and 11.8 s with 16 test threads, and 8.5 and 8.7 s with
4, while the whole suite took 13% longer. Which other work slowed the
failing gates is not known, and no gate has yet shown that the cap is
enough.
