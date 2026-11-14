# climon2 work-stop handoff (2)

Snapshot: 2026-11-14 ~17:10 UTC. The owner asked, through the climon2 session `climon2-f8`, for a graceful stop and this handoff: no new jobs, merges or pushes to PR branches. The previous stop and resume are in `../monitor-stop/`.

## Stop follow-through
- **Agents:** the two still-active subagents (#109, #202) acknowledged and are idle. Their state is below. No other agent was running.
- **Gates:** #88's ticket gate `t88-tk2` was running. It was stopped incomplete, and its log ends with `GATE EXIT stopped-by-agent (owner requested stop; incomplete)`. The queued #157, #171 and #84 gates were cancelled before they started. `flock -n /tmp/climon2-monitor-gate.lock true` succeeded.
- **Watchers:** the CI watchers for #159 and #162 were stopped.
- **GitHub CI was left running**, not cancelled. The queued runs are only checks, and their results are useful evidence for the next landing step:
  - #159 at `6fd3c8d`;
  - #162 at `0eac2b9`;
  - #202 at `8f6db3c` (run 43616633890).
- **Merges:** none during the stop.
- **Empty log:** `climon2-f8` left an empty `t84/target/gate-logs/climon2-gate-t84-tk1.log` (its duplicate gate was killed before starting). It is not a result; the next #84 gate may overwrite it.
- **Worktrees:** preserved, including dirty state. t76, t77 and t88 hold only untracked `crates/climon2-cli/default_*.profraw` coverage files. t88 also has two leftover daemon test temp directories, `crates/climon2-daemon/tests/daemon-*`. All are disposable gate leftovers, not work.

## Where things stand
- Repo `/home/jack/dev/climon2`; integration branch `feature/monitor-impl` is at `6930b0e`; `main` is at `ed94bf1`. Effort #73. The monitor POC path ends at #109.
- **Landed this session:**
  - into `main`: #183 (`.claude/skills` symlink), #185 (tiered quality gate), #186 (mutation history cleared before every run);
  - main synced into `feature/monitor-impl` as `ed27e70`;
  - into `feature/monitor-impl`: #176 (#85, `c6c88f3`) and #173 (#105, `6930b0e`). Issues #85 and #105 are closed.
- `main` has since gained `ed94bf1` (#200, glossary). It is not yet merged into `feature/monitor-impl`; sync it with owner approval.
- Per-job state: `climon2-handoff-pr-*.md` (open PRs) and `climon2-handoff-stacked.md` (#83, #89, #93, #94, #106, #108, #109, which have no PRs yet).
- Session coordination files: `/home/jack/.copilot/session-state/d74b2ba9-7532-4c59-82a4-44f31fe3a72e/files/`. That holds `agent-rules.md` (binding; attribution rule updated), `log.md` (append-only decision log), the `codereview-*.md` files (#94, #106 and earlier) and the per-ticket `log-t*.md` files.

## Landing rule (owner, 2026-11-14)
- Land every open PR except #123 (`feature/monitor-impl` → main) and #72 (do-not-merge prototype) once it is safe:
  - its gate has passed;
  - CI is green on the head that already contains the current `feature/monitor-impl`;
  - no owner question is open.
- Land one at a time, squash-merged into `feature/monitor-impl`.
- **No Claude/AI attribution** in commits, squash messages or PR bodies (owner rule; also in memory). Write an explicit `--body-file`. GitHub's default squash body concatenates commit messages, some of which carry old trailers.
- After each landing, merge the new `feature/monitor-impl` into the next branch. Re-gate a branch only if a non-Markdown conflict was resolved by hand. Git-clean or Markdown-only merges need only CI on the merged head.
- Close the ticket issue with a comment giving the merge commit, the gate evidence and the CI run.
- Background `gh pr merge` was blocked by Claude Code's auto-mode classifier once; foreground merges after the owner's go-ahead worked.

## Gate (one at a time on this host)
- Environment: `export PATH="$HOME/.cargo/bin:$PATH" CLIMON2_TEST_BASH=/usr/bin/bash`. Run `cargo xtask setup-quality` once per worktree. Rust 1.96.0.
- Ticket gate (since #185, the default):
  ```
  cd <worktree> && rm -f crates/climon2-cli/default_*.profraw && mkdir -p target/gate-logs && L=target/gate-logs/climon2-gate-<t>-tk<N>.log && CLIMON2_MUTATION_BASE=origin/feature/monitor-impl PYTHONUNBUFFERED=1 flock /tmp/climon2-monitor-gate.lock cargo xtask check > $L 2>&1; echo "GATE EXIT $?" >> $L
  ```
  - It runs 1 load round, then mutation of only the functions changed since the merge base. `cargo xtask check --print-mutation-plan` shows the selection.
  - The branch must contain #185/#186 (any branch that merged `feature/monitor-impl` at `ed27e70` or later does).
- Full gate: `cargo xtask check --full` with `CLIMON2_MUTATION_BASE=origin/main`. It runs once, before #123 merges to main.
- **Other cargo commands** go through `bash $S/test-slot.sh cargo …`, where `$S` is the session-state files directory. At most 3 run at once host-wide.
- **Stress runs** go under the gate lock.
- **Stopping a gate:** kill the gate's process tree, confirm the lock is free, and append a `GATE EXIT stopped-by-…` line to its log.

## Open owner questions
None pending. Every question raised this session was answered and recorded on its issue (#88, #94, #106, #108, #109, #120, #112, #84). New follow-up tickets filed this session:
- #184: an attachment lost to output overflow never inherits control (blocked by #89);
- #203: Command final waits for in-progress restorations and held synchronized updates (blocked by #88).

## Suggested skills
Read `.agents/skills/` (also visible as `.claude/skills` in checkouts that contain #183): `ub-mode` and `tdd` before implementation, `diagnosing-bugs` for gate or CI failures, `code-review` before proposing a merge, `pr` for PR bodies, `protocol-docs` for docs/protocol changes, and `grilling` (never answer its questions on the owner's behalf).
