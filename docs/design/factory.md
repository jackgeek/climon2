# Factory design

The factory is a local Python program that builds climon2, a cross-platform Rust terminal-session tool, one feature at a time. Each feature gets approved end-to-end acceptance tests before any implementation exists. Terminology follows [`CONTEXT.md`](../../CONTEXT.md); key decisions are recorded in [`docs/adr/`](../adr/).

## Goals

- Deliver climon2 feature by feature, each proven by black-box acceptance tests on Windows, Linux and macOS.
- Keep the user in control: every contract, test suite, macOS execution and merge is explicitly approved.
- Make agent output trustworthy: tests are approved before code exists, proven to fail for the right reason, and cannot be edited by the Implementer.
- Never run agent-written code on the host without isolation or explicit approval.

## Repository layout

One repository (`jackgeek/climon2`) holds the factory, the acceptance harness, the feature artifacts and climon2 itself.

```
CONTEXT.md                      Glossary (ubiquitous language)
docs/
  adr/                          Architecture decision records
  design/                       Design documents (this file)
  research/                     Cited research notes
factory/                        The factory (Python)                  [protected]
  pyproject.toml
  src/climon2_factory/          Factory source
  roles/<role>.md               Role prompts
  skills/<role>/                Curated skills given to each role
  containers/Dockerfile         Pinned build/test image
  tests/                        Factory tests (deterministic, fake agents)
conftest.py                     Loads the harness fixtures for every acceptance test [protected]
harness/                        Acceptance-test harness (pytest)      [protected]
  README.md                     The harness API, written for the Test Author
  terminal.py                   Real PTY (Unix) and ConPTY (Windows, pywinpty) terminal driver
  fixture_program.py            Generic terminal fixture program
  plugin.py                     pytest fixtures (install layout, home, env, terminals)
  tests/                        Harness self-tests, run on all three platforms
features/
  ROADMAP.md                    Ordered features and dependencies     [protected]
  <feature-id>/
    brief.md                    User prose given to the Contract Author
    contract.md                 Feature contract                      [protected once approved]
    findings/                   Reviewer findings and dispositions
    tests/                      Acceptance tests                      [Test Author only]
climon2/                        Climon2 (Rust)
  Cargo.toml, Cargo.lock, rust-toolchain.toml
  src/
.github/workflows/              CI                                    [protected]
.factory/records/<artifact>/    Append-only manifests and approval decisions [protected]
```

Protected paths can be changed only by the user through normal commits, never by an agent during a feature run. The factory cannot modify itself during a feature run.

Run state lives outside the repo, in `~/.climon2-factory/runs/<run-id>/`. It holds the workflow checkpoint, transcripts, telemetry and evidence. Native macOS runs use `~/.climon2-factory/macos-runs/<hash>/`.

## Climon2 shape (as specified so far)

- **Executables:** each runtime version is self-contained: `versions/<v>/{climon2-cli, climon2-daemon, climon2-client}`. The stable `climon2` launcher reads `current`, which holds the current version, and runs that version's CLI. See [ADR-0005](../adr/0005-launcher-and-versioned-runtimes.md).
- **Stable surfaces:** four interfaces must remain backward compatible:
  - the session metadata format
  - the hello handshake
  - the client launch interface (`climon2-client --protocol <n> attach --metadata <path>`)
  - the launcher contract
- **IPC:** each session daemon listens on a Unix domain socket on every platform. Access is restricted to the same user with a user-only runtime directory and a peer UID/SID check. See [ADR-0003](../adr/0003-unix-domain-sockets-everywhere.md).
- **Session lifetime:** a detached session daemon exists from feature 1. `exec` launches the daemon, then attaches as its first client. See [ADR-0004](../adr/0004-detached-daemon-from-feature-1.md).
- **Discovery:** session metadata lives at `<climon2 home>/sessions/<guid>.pty`. Climon2 home defaults to `~/.climon2` and can be overridden with `CLIMON2_HOME`. A session is listed only if its daemon answers the hello handshake with the matching session ID.

## Roles

| Role | Model | Writes | Purpose |
|---|---|---|---|
| Contract Author | `gpt-6-astra` | `features/<id>/contract.md` (draft) | Drafts a contract from the brief and surfaces questions |
| Test Author | `gpt-6-astra` | `features/<id>/tests/`, scaffold in `climon2/` | Acceptance tests, negative controls, unimplemented scaffold |
| Implementer | `gpt-6.1-sol` | `climon2/` (not tests) | Makes the acceptance tests pass |
| Reviewer | `claude-opus-5.5` | `features/<id>/findings/` | Adversarial, read-only review in a fresh session each time |

A role's model must be available. The factory never substitutes another model silently.

### Agent runtime

- Agents run through Microsoft Agent Framework's `GitHubCopilotAgent` (`agent-framework-github-copilot`).
- **Tools:**
  - All built-in Copilot tools are denied: no shell, URL fetching, or unscoped file access. Two layers enforce this: the session's `available_tools` allow-list holds only the role's tools, and the permission handler approves only those custom tools and refuses everything else.
  - Tool names carry a `factory_` prefix. An allow-listed name also enables any same-named host tool (Copilot or IDE), which shadows ours and escapes the path checks.
  - Read tools (`factory_read_file`, `factory_list_files`, `factory_search`) are given to every role. Output stays under 16 KB, below the 20 KiB at which Copilot moves a result to a temp file the agent can't read, so long files are read a page at a time. Write tools (`factory_write_file`, `factory_replace_text`, `factory_delete_file`) are scoped to the role's paths, and `.git` is never reachable.
  - `factory_update_lockfile` (Test Author, Implementer) regenerates `Cargo.lock` in a networked container, so dependencies can be added without giving agents network access. `factory_run_acceptance_tests` (Implementer) runs the per-attempt budgeted Linux build and tests.
  - Each role gets path-scoped custom tools that the factory enforces in Python. Paths are resolved through `..` and symlinks before checking.
  - Agents have no network access, credentials or tokens. Pushing and calling CI are factory functions, never agent tools.
- **Prompts:** `factory/roles/_common.md` plus `factory/roles/<role>.md`, including the contract format (numbered `B` requirements, `S` scenarios citing them) and the scaffold and negative-control rules.
- **Context:** each agent starts with a small bundle and can read more on demand.
  - The bundle contains the role prompt, `CONTEXT.md`, the relevant approved artifacts, and a repository map.
  - Retries receive the diff, failures and findings, never earlier transcripts.
- **Skills:** each role gets curated, protected skills from `factory/skills/<role>/`:
  - Contract Author: domain-modeling and an adapted grilling skill
  - Test Author: tdd
  - Implementer: codebase-design and diagnosing-bugs
  - Reviewer: codebase-design
- **Completion:** every agent step must end with a typed `factory_submit_*` tool call. Ending without one fails the step.
- **Limits:** each agent invocation is limited to 60 minutes; CI waiting time doesn't count. Only one feature run can be active at a time.
- **Findings:** each finding is either blocking or advisory. The user can override any finding.

## Feature run

```
brief ─▶ Contract Author ─▶ Reviewer (≤2 rounds) ─▶ ✋ approve contract
      ─▶ Test Author (tests, negative controls, fixture use, scaffold, red evidence)
      ─▶ Reviewer ─▶ ✋ approve tests + red evidence
      ─▶ Implementer (≤3 attempts × 60 min, ≤10 container runs per attempt)
      ─▶ candidate: Linux container gate + GitHub Windows (MSVC)
      ─▶ ✋ approve macOS run of this commit ─▶ native macOS gate
      ─▶ Reviewer ─▶ factory opens PR ─▶ ✋ user merges
```

Rules:

- **Contracts:** a contract is structured Markdown with these sections:
  - Purpose
  - numbered Behavior requirements
  - Given/When/Then Scenarios that cite those requirements
  - Platforms/shells
  - Out of scope
  - Depends on

  The factory checks traceability mechanically (`traceability.py`) and adds each problem as a blocking finding beside the Reviewer's:
  - every section is present;
  - requirements (`- **B1.** …`) and scenarios (`- **S1** (B1, B2). …`) are numbered without gaps;
  - every scenario cites existing requirements, and every requirement is cited by a scenario;
  - at tests review, every scenario is cited (`S<n>`) by at least one test file.
- **Regression:** the acceptance tests of every previously delivered feature are permanent regression gates.
- **Amendments:** changing a delivered contract requires a contract amendment, approved like a new contract.
- **Red evidence:** before implementation starts, the factory runs the tests against an explicitly unimplemented scaffold.
  - Every test must fail.
  - Every negative control must fail for its intended reason.
- **Implementer boundaries:** the Implementer can read tests but never write them. Test-only paths in the product are forbidden.
- **Failure handling:** three failed attempts pause the run for the user.
- **Rejections:** a rejection sends the artifact to a fresh session of the same role, followed by another review. There is no cap on revisions.
- **Direct edits:** the user may edit artifacts directly, and edits are still reviewed.
- **Upstream changes:** if an approved artifact changes mid-run, the run returns to it and all downstream work (attempts, candidate) is redone. A direct edit is recorded as the user's revision and goes to review (tests get a fresh red run first). An artifact made stale by changed inputs goes back to its author, with the changed inputs as findings. `status` names the changed artifact and inputs, and approvals are refused until `factory resume`. Once the delivery PR is open, the PR review takes over.

### Factory CLI

`factory start <feature-id>`, `status`, `show <artifact>`, `approve <artifact> <hash>`, `reject <artifact> --reason …`, `resume`, `abort`, `release <version>`.

Each command advances the run to its next pause and exits. Pauses print the next step and raise a macOS notification.

Run it from anywhere in the repository with `uv run --project factory factory <command>` (or `uv run factory …` inside `factory/`). It works on the repository containing the current directory and keeps run state in `~/.climon2-factory/`. Docker and the macOS toolchain are only set up when a step first needs them.

### Artifacts and approvals

See [ADR-0001](../adr/0001-content-addressed-artifacts.md).

- **Content addressing:** every artifact is content-addressed. Its manifest records the hashes of its inputs, including role prompts and skills.
- **Staleness:** validity is always recomputed and never cached. An artifact whose recorded inputs no longer match is stale and cannot advance the run.
- **Approvals:** an approval binds to one artifact hash as recorded with its inputs, so re-recording (even with identical content) needs a fresh decision. Superseded artifacts are kept as history.
- **Storage:** contracts, findings, approvals and tests are stored in the repository. Each artifact (named like `<feature-id>/contract`) has append-only JSON manifests and decisions under `.factory/records/<artifact>/`. Run state is stored locally.
- **Path safety:** artifact paths must stay inside the repository and may not contain or pass through symlinks.

### Persistence and recovery

- **Checkpoints:** workflow state is checkpointed after every step.
- **Interrupted agent steps:** after a crash, an interrupted agent step is discarded and rerun. Attempt and time counts that were already consumed are not refunded.
- **External side effects:** pushes, CI runs and PRs are recorded as intents before they happen, then reconciled against GitHub when the run resumes. The stage itself is the intent: candidates go to a deterministic `factory/<run-id>-a<attempt>` branch (re-pushing is harmless, since CI results are checked against the new commit), and the PR for that branch is looked up before one is opened.
- **Failed steps:** a step that fails (e.g. CI reports the wrong commit) leaves the run at that stage; `factory resume` retries it.

## Execution environments

Agents never execute code on the host. See [ADR-0002](../adr/0002-agents-never-execute-on-host.md).

| Where | Used for | Evidence label |
|---|---|---|
| Local Rancher Desktop containers (dockerd) | Inner loop, Linux gate, Windows GNU cross-compile check | `local-container` |
| GitHub Actions `windows-2025` | Windows gate for declared candidates (MSVC) | `github` |
| GitHub Actions full matrix | Release only | `github` |
| Native macOS (this Intel Mac) | macOS gate, after per-commit user approval | `macos-native` |

- **Containers:**
  - Each run uses a fresh container built from a factory-owned, pinned Dockerfile (Ubuntu 24.04, pinned Rust, `x86_64-pc-windows-gnu` with MinGW, Python and pytest, bash/sh). The image is referenced by digest.
  - The worktree is copied in. There are no mounts except a read-only cargo cache, which is filled by a separate networked `cargo fetch --locked` step.
  - Containers run with `--network none`, no credentials, and CPU, memory and time limits.
  - Results are re-parsed outside the container as untrusted input. The expected tests are collected first in a separate container that never builds or runs product code. The JUnit report is then cross-checked against that list and against pytest's real exit code: missing, duplicate or unknown results fail, skips and errors fail, and a non-zero exit with a clean report fails every test. Known gap: product code runs as the same user as pytest, so it could still turn a skip into a pass. Running the product as a separate user closes this, and is part of the harness (build step 5).
  - Run contract: the subject (`climon2/`, or a negative control directory) is a Cargo project built with `cargo build --release --locked --offline`. Tests find its binaries through `CLIMON2_BIN_DIR`. Test IDs are pytest node IDs (`features/<id>/tests/test_x.py::test_y`), so a negative control's target test is a node ID. A build failure is reported as a `build` result, with every test "not run".
  - Crates: a separate networked container runs `cargo vendor --locked`, which downloads sources but builds nothing. It fills a Docker volume keyed by the `Cargo.lock` hash, which is mounted read-only into test containers.
  - The workspace is copied in as root-owned files, excluding `.git`, `.factory`, `factory/`, `target/` and virtualenvs. Tests and product code run as an unprivileged user with all capabilities dropped.
- **GitHub:**
  - Only the factory pushes, only to `factory/*` branches, and only for declared candidates.
  - CI uses a read-only `GITHUB_TOKEN` and no secrets.
  - Agents cannot change workflows, the harness or the factory.
  - The factory verifies that CI results belong to the expected commit hash.
  - A candidate is the workspace committed on top of HEAD through a temporary index, so the user's HEAD, index and working tree are untouched. It is force-pushed, so re-pushing a branch on resume replaces the earlier candidate.
  - Pushing to `factory/**` triggers `.github/workflows/factory-windows.yml`. It collects the expected tests before anything is built, then builds `climon2/` with `cargo build --release --locked` and runs every `features/*/tests`. The collected list, build result, JUnit report and exit codes are uploaded as artifacts. The factory finds the run by commit and workflow path, then applies the same untrusted-report cross-checks as the containers. Missing artifacts fail every test with the run's conclusion.
- **macOS:**
  - The user approves each commit after the Windows and Linux gates pass. The factory highlights build-time code changes (`build.rs`, proc macros, Cargo files, new dependencies).
  - The run uses a fresh checkout of the commit (`git archive` from the local repo, so unpushed and unreferenced candidate commits work) in `~/.climon2-factory/macos-runs/<commit>/`, which is deleted afterwards.
  - The environment is built from scratch, not inherited: a temporary HOME, a minimal PATH, and a factory-owned toolchain in `~/.climon2-factory/toolchain` (rustup-init pinned by hash, the pinned Rust, and pytest installed from the hashed container requirements).
  - Only the requested features' tests are collected, before anything is built. Crates are fetched with `cargo fetch --locked`, then built with `cargo build --release --locked --offline`. The report goes through the same untrusted-report cross-checks.
  - Each step runs in its own process group. The run has a 30-minute limit, after which the whole group is killed. Afterwards, any of the user's processes whose working directory is still inside the run directory are killed (found with `lsof`; macOS hides process environments, so an environment marker can't be used). Known gap: a process that changes directory elsewhere, or reaches the network, escapes, until a macOS VM is evaluated.
  - Unattended macOS runs are deferred until a macOS VM is evaluated (see [research](../research/macos-execution-isolation.md)).
- **Toolchains** are pinned everywhere. Only x64 is supported. Shipped Windows binaries are MSVC builds from GitHub; the GNU target is used only for early compile checks.

## Acceptance harness

- **Framework:** Python pytest with real terminal drivers: a PTY on Unix and ConPTY on Windows. The terminal fixture runs alongside every required shell test.
- **Required shells:**
  - Windows: `powershell.exe`, `pwsh.exe`, `cmd.exe`
  - Linux: `bash`, `sh`
  - macOS: `zsh`, `bash`, `sh`

  Shells must be installed; missing ones are never substituted. Shell startup is controlled, and each shell gets an interactive smoke test (marker, discovery, exit, cleanup).
- **Real-terminal proof:** tests use native terminal checks, verify terminal dimensions, and complete an end-to-end exchange through the session.
- **Packaging:** tests run against the extracted release-format package through the launcher. Paths with spaces and Unicode, and exact argument preservation, are covered.
- **Strictness:**
  - Readiness waits on observable conditions with approved deadlines.
  - Leaked processes fail the test.
  - A test that passes on rerun still counts as failed.
- **Cross-user rejection:** this test runs on every platform. On the Mac it needs a hidden standard user, `climon2-test`, created once by a user-run setup script with a narrow sudoers rule. If that user is missing, the gate fails.
- **Clean validation:** final validation uses a fresh workspace, verified to be at the candidate's commit.
- **Provenance:** the exact tested binaries are retained with provenance. Releases are unsigned for now.
- **Implementation:** see `harness/README.md`. The root `conftest.py` loads `harness.plugin`, so every gate (container, Windows CI, native macOS) gets the same fixtures. pywinpty is pinned by hash for Windows in the universal `factory/containers/requirements.txt`.
- **Harness self-tests:** `harness/tests/` checks the drivers, the fixture program and every required shell. Run them from the repository root with `factory/.venv/bin/python -m pytest -p no:cacheprovider --rootdir=. harness/tests`. They were validated on all three platforms (macOS natively, Linux in the factory image, and Windows Server 2025 CI through a throwaway `factory/` branch).
- **Known gap:** tests and product run as the same user, so cross-user rejection can't yet be proven from inside a gate. Product code could also turn an expected failure into a skip. Both are deferred.

## Releases

`factory release <version>` (SemVer, starting at 0.1.0):

1. Runs the full GitHub matrix and the approved native macOS gate.
2. Has an agent draft a [Keep a Changelog](https://keepachangelog.com/) entry from contracts and amendments, with breaking changes called out. The user approves it.
3. Publishes the exact gate-tested packages, checksums and manifest as a GitHub Release.

From 0.2.0, release gates include cross-version tests: sessions from the previous release must work with the new launcher and CLI.

## Observability

- **Transcripts:** full agent transcripts are kept.
- **Telemetry:** OpenTelemetry output is written to local JSONL, and `factory status` summarizes it.
- **Limits:** attempt and time limits apply; there are no cost budgets.

## Building the factory

The factory is built conversationally and test-first, through pull requests, in this order:

1. Artifact store (content addressing, manifests, staleness, approvals)
2. Workflow and CLI driven by scripted fake agents
3. Executors (containers, GitHub, native macOS)
4. Real agents (`GitHubCopilotAgent`, scoped tools, submit tools)
5. Harness foundation: PTY/ConPTY drivers and the terminal fixture, validated once manually on GitHub Windows

**Stack:** Python 3.12, uv, pytest, pyright (strict on core) and ruff.

- Factory tests run locally and are deterministic. A separate smoke test exercises real Copilot.
- Validation command (from `factory/`): `uv run pyright && uv run pytest && uv run ruff check && uv run ruff format --check`
- Executor tests need real infrastructure and are opt-in: `uv run pytest -m container` needs Rancher Desktop running with the moby engine (`~/.rd/docker.sock`). `uv run pytest -m github` needs `gh` logged in with push access to climon2. It pushes throwaway `factory/test-*` branches to climon2 itself, waits for real Windows CI, opens and closes a PR, and deletes the branches afterwards.
`uv run pytest -m copilot` runs real Copilot agents (Contract Author and Reviewer) in a throwaway repository, using premium requests; it needs the `copilot` CLI logged in.
`uv run pytest -m macos` runs the native macOS gate on this Mac; the first run installs the factory toolchain (about 30 seconds).
