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

## What a gate mutates

Each gate mutates only the functions a push adds or changes, as the local
ticket gate does ([The quality gate](development.md#the-quality-gate)). Before
[#445](https://github.com/jackgeek/climon2/issues/445) it mutated every
function in every changed file. So three edited functions in a 2,000-line
file kept the Linux gate busy for one to two hours.

- **Changed functions.** A function counts as changed if it is new, or if its
  code differs from the base's. Whitespace and comments don't count.
- **Its dependents.** A changed `struct`, `enum`, `const`, `static` or `type`
  can change how functions that didn't change behave. So it also selects every
  function in the same file whose code names it, counting the header of the
  `impl` the function is in. For example, a new field in `Frame` selects every
  method in `impl Frame`. The rule goes one step only: a function chosen this
  way does not choose the functions that call it. Deleting or renaming one
  selects the functions that used it. A name written inside a format string,
  such as `"{LIMIT}"`, counts, and so do the names a `use` imports from it, so
  changing `Mode` selects the functions that use `A` after `use Mode::{A, B}`.
- **Imports.** Each name a `use` imports is compared on its own. A name that
  is added, removed or imported from somewhere else selects, in the same way,
  every function that uses it, by its original name or its `as` name. Adding
  `Placeholder` to `use screen::{Canvas, Scene}` selects only the functions
  that name `Placeholder`. If no function names a changed import, before or
  after the change, it is most likely a trait whose methods are called, and
  those calls can't be found by name, so the whole file is mutated.
- **Tests.** A test that is added, removed or changed (`mod tests`, `#[test]`
  or `#[cfg(test)]`) selects the functions in the same file that the test's
  code names, methods included, under its old and its new version. A
  weakened or removed test therefore still has to kill the mutants of the
  functions it covered. If a changed test names no function of the file, the
  whole file is mutated. The local ticket gate does not apply this rule.
- **Whole files.** A new file is mutated whole. So is a file in any of these
  cases:
  - its base version is not valid Rust;
  - it adds, removes or changes a glob import (`use x::*`) or an import as
    `_`, whose names can't be found;
  - it has a glob import of a changed or deleted item, such as `use Mode::*`
    when `Mode` changed;
  - it changes code that has no dependents rule yet: a `macro_rules!` or
    other item macro, a trait's method signature or associated type, an
    impl's associated type, a `union` or an `extern` block.
- **CRAP is not narrowed.** Every function of the source the gate scores is
  still scored, whether or not it changed. A native gate scores its changed
  native files even when their changes select no mutant, for example a
  comment-only edit.

The gate prints its choice before it mutates. It lists each file, its
selected functions and their lines, and the mutants it will try. A whole file
stays in the list even if no mutant is found in it. If the mutator's scan
doesn't report a file under the same path as the gate, the gate fails, so a
file can't be skipped without anyone seeing it. The count is
taken before coverage, so mutants on lines no test runs are not tried.
`cargo xtask check --print-mutation-plan` prints the same choice locally,
with a [`CLIMON2_MUTATION_BASE`](development.md#the-quality-gate) set.

**One run, so one baseline.** A gate starts the mutator once for every file.
Each run first passes the whole unchanged workspace once (the baseline), which
takes minutes, so one run per file would repeat it. But the mutator takes only
one list of lines for all its files. So the gate gives it every line it
selected in any file, and a file can also be mutated on a line number that
was chosen in another file. That only ever tries more mutants, never fewer,
and the plan shows how many each file gains this way. A new file adds all its
lines to the list, so it can gain the most. Until the mutator takes a list per
file, the owner keeps this (2026-11-27).

## Skipping a squash-merge's gates on main

A pull request's gates run on its branch's push, and they run again on the
push its merge makes to `main`. The Linux gate can take hours on the one Linux
machine, so that second run delays every other branch
([#442](https://github.com/jackgeek/climon2/issues/442)).

A squash-merge of a branch that is up to date with `main` gives `main` the same
**tree**, the snapshot of every file, as the branch's last commit. So on a
push to `main`, a job on a GitHub-hosted runner runs
`tools/quality-prior-run.py` before the gates start. The script looks through
the last 100 successful `quality.yml` runs for a push to another branch whose
commit has that tree. When it finds one in which the Linux, Windows and macOS
gates all passed, the three gates on `main` are skipped. The run's summary
names the tree and the run it relied on.

The gates on `main` still run in every other case:

- a push to any other branch, or a manual run of the workflow;
- a push to `main` whose tree no passed branch run had, such as a direct push
  or a merge of a branch that was not up to date;
- a branch run of that tree in which a gate failed or was skipped;
- the look-up failing, for example because GitHub's API did not answer.

The branch's gate mutated at least the source that `main`'s push changes: the
two trees are the same, and the branch's gate compared against an older or
equal `main`.

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
