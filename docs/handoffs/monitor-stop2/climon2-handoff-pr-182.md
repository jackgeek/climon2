# Handoff: PR #182 / #88 Command final and standalone Kill over daemon wire revision 1

Worktree `t88`, branch `impl/88-command-final-and-standalone-kill`, head `e408a5c` (pushed). Merge pre-approved once the gate and CI pass.

- **Owner decision 2026-11-14, revising 11-13 "timing kept":** Command final is sent only after the command's output has drained.
  - Implemented in ad30207 and e408a5c.
  - The drain ends at the first of: PTY EOF/EIO, 100 ms idle, or a 1 s cap after the reap.
  - On a hang-up, the final goes out at the end of the 5 s grace.
  - The test `command_final_follows_the_commands_last_output` was red first.
- **Load-flake test fixes:** 90b093e fixes the focus-ordering test and replaces the 7 s grace upper bound with a timing-free oracle.
- **Gate:**
  - `t88-1` (full) failed on those load flakes, now fixed.
  - `t88-tk1` was stopped for the timing change.
  - `t88-tk2` at `e408a5c` was stopped by the owner's stop: incomplete, 27 killed and 0 survived so far. **Not a pass.**
- **Next:**
  1. Run the ticket gate at `e408a5c` (the branch contains `ed27e70`, so it uses the tiered gate).
  2. Merge `feature/monitor-impl`. It conflicts in `docs/protocol/README.md` (Markdown only), so no re-gate is needed.
  3. CI, then squash-merge, then close #88.
  4. Then #89 (stacked) merges #88's final head. Expect conflicts at #89's old `publish()` call sites in session.rs; they are resolved by removing those calls.
- **Follow-up:** #203 (final waits for in-progress restorations and held sync updates).
- Dirty: cli profraw files and `crates/climon2-daemon/tests/daemon-*` temp dirs left by the stopped gate (disposable).
