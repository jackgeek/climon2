# Session continuity: implementation handoff

## Start here

Implement the agreed next-release session-continuity behavior in `jackgeek/climon2`.
The user authorized implementation, then requested this handoff for a new agent.
No product implementation was started in the originating session.

Read `AGENTS.md`, invoke `ub-mode`, and follow the issue/domain pointers there.
Before this handoff, `main` was at `79202a5`; the only worktree change was the
user's unrelated `CLAUDE.md` modification. Recheck the worktree and preserve that
change outside your commits.

The user may be unavailable. Choose the simplest internals consistent with the
settled behavior. Record genuine blockers and continue independent work; ordinary
implementation design does not require another interview.

## Authoritative scope

Read these **latest amendments before older issue bodies or prototype findings**:

- [#40 scope index](https://github.com/jackgeek/climon2/issues/40#issuecomment-6552102664).
- [#43 engine/testing correction](https://github.com/jackgeek/climon2/issues/43#issuecomment-6552082936).
- [#48 restore/hyperlink correction](https://github.com/jackgeek/climon2/issues/48#issuecomment-6552082933).
- [#52 native-testing correction](https://github.com/jackgeek/climon2/issues/52#issuecomment-6552082929).
- [#45 harness amendment](https://github.com/jackgeek/climon2/issues/45#issuecomment-6552082952).

The positive target is ordinary, working session continuity with automated product
sanity checks:

- **Alacritty is final.** Its existing upstream tests need not be exhaustively
  repeated in climon2. Use focused tests at climon2's own interfaces.
- Preserve normal screen/history restoration and ordinary input/control behavior.
  Detailed upstream fidelity and nonstandard cursor-shape investigations are not
  prerequisites.
- Hyperlinks may become plain text instead of failing the session. Emit retained
  links safely using the already-approved mechanical control-byte encoding.
  Detailed hyperlink FIFO/accounting proposals were subsequently withdrawn; they
  are not a finalized new acceptance specification.
- Native character presence and simple product sanity checks suffice. Exact
  palette, glyph-outline/accent-shape and elaborate per-character observer
  qualification are outside the revised gate.
- Default title reflects the executed command, for example `climon2 exec bash`.
  Preserve ordinary Unicode titles and percent-encode embedded C0/DEL/C1 bytes.
- Bash **5.x only** is qualified interactively. Command execution remains
  shell-agnostic; installer and non-interactive `/bin/bash` compatibility remain.
- Required native hosts remain Terminal.app and VS Code, with the previously
  agreed native arm64/Intel scope. Existing packaged compatibility and quality
  gates remain.

Historical prototype failures retain their original evidence status. Scope
reduction is not evidence that those cases passed.

### Behavioral sources

Read the resolutions and subsequent amendments, not just the initial questions:

| Work | Source |
|---|---|
| Attach selectors, TTY preflight, JSONC configuration, escaping/passthrough, results and feedback | [#41](https://github.com/jackgeek/climon2/issues/41#issuecomment-6473957390) |
| Explicit detach survival, unexpected final-client loss, initial attachment, natural completion and deliberate kill | [#42](https://github.com/jackgeek/climon2/issues/42#issuecomment-6474530309) |
| Version-matched dispatch, optional capabilities/live state and frozen client startup contract | [#44](https://github.com/jackgeek/climon2/issues/44#issuecomment-6547968217) |
| Takeover/hand-back, control periods, stale input, readiness, placeholder and terminal sizing | [#47](https://github.com/jackgeek/climon2/issues/47#issuecomment-6476116795) |
| Native scrolling, retained history, restore/catch-up and host preservation boundaries | [#49](https://github.com/jackgeek/climon2/issues/49#issuecomment-6477556657), [history amendment](https://github.com/jackgeek/climon2/issues/49#issuecomment-6544545933) |
| Identified direct self-attachment rejection only; 125 without disruption | [#50 replacement](https://github.com/jackgeek/climon2/issues/50#issuecomment-6548465337) |
| Existing approved ordinary numerical budgets | [#48 budget decisions](https://github.com/jackgeek/climon2/issues/48#issuecomment-6546525973), read with its scope correction above |

Unidentified ancestry alone does not refuse attachment. The monitor, global cycle
detection, extended keyboard/application-keypad fidelity, crash/reboot recovery
and completed-session persistence remain outside scope.

## Implement in verifiable increments

1. **Establish the baseline.** Read the sources above, then trace the existing
   CLI/daemon/client and tests. Select a verified bash 5.x executable through
   `CLIMON2_TEST_BASH`. Run the existing checks appropriate to the first increment.
   Completion: existing behavior and any environment failures are understood.

2. **Make configuration and protocol decisions explicit.** Implement the
   agreed CLI/configuration and frozen startup/capability contracts; define only
   the version-specific messages needed for this release. Keep input/control/
   resize/restore ordering local to one daemon-owned state machine.
   Completion: focused tests prove admission, configuration snapshots and message
   ordering without changing frozen framing, metadata or client argv.

3. **Add the daemon-owned virtual terminal and restoration.** One PTY output
   pump updates Alacritty and delivers client output. Clients keep no virtual
   terminal. Add history and normal/alternate restoration with an ordered handoff
   to live output. The simple serialization approach is an implementation choice;
   no particular prototype strategy or trial delay/buffer was approved.
   Completion: product tests restore ordinary shell/TUI state and continue input
   and output afterward; daemon query responses work without helper-made replies.

4. **Complete lifetime and multi-client behavior.** Add attach/detach/kill,
   explicit-detach survival, unexpected final-loss termination, takeover/hand-back,
   stale-input isolation, readiness and controlling-client geometry.
   Completion: focused PTY tests demonstrate the agreed outcomes, including
   output while detached and another client remaining after departure.

5. **Exercise installed product paths.** Extend the existing packaged tests,
   controlled bash profile and generic/TUI fixtures. Use fixture acknowledgements
   or observable completion for ordering; sleeps are not proof of readiness.
   Completion: installer -> launcher -> CLI -> daemon -> client tests cover
   continuity, actual exit status and owned-resource cleanup.

6. **Finish delivery.** Run the existing quality/release checks applicable to
   the changes, plus simple native sanity checks where available. Update affected
   usage docs, shell matrix, architecture records and dependent call sites.
   Completion: passing work is committed/pushed; report exact completed behavior,
   outstanding environment/architecture gates and the action log.

Use red-green tests for each behavior change. Fix real product failures rather
than reopening the abandoned engine/renderer research.

## Existing seams and important gotchas

- Startup: `crates/climon2-cli/src/exec.rs`; daemon resources:
  `crates/climon2-daemon/src/startup.rs`.
- PTY ownership/pump: `crates/climon2-daemon/src/{pty,session}.rs`.
  Today the pump forwards output only to one controller and has no screen/history.
- Client handshake/raw-mode/byte bridge:
  `crates/climon2-client/src/{lib,transport,bridge}.rs`.
- Shared neutral contracts: `crates/climon2-core/src/{cli,protocol,metadata,layout}.rs`.
- Test terminals/fixtures: `crates/climon2-test-term` and
  `crates/climon2-fixtures`; installed tests:
  `crates/climon2-e2e/tests/{packaged.rs,support/mod.rs}`.
- The existing helper already uses Alacritty and automatically answers
  `Event::PtyWrite` queries. Disable/bypass those automatic replies in tests of
  daemon-owned query handling and assert independently specified response bytes.
- The helper's crates.io Alacritty dependency is not a shipped-daemon dependency.
  Embedding evidence uses development pin
  `bbb2cbf9470a6707b1205fbddbdb25109b14e2fd`; consult #43/#56 when selecting
  compatible dependency declarations. Prototype branches are reference assets,
  not product implementations to merge wholesale.
- ADRs 0001-0005 exist locally. #44 refers to ADR 0006, which was absent from
  this baseline; reconcile the authoritative startup contract when documenting it.
- Respect the shipped/test-only dependency directions in ADR 0004. If a shipped
  VT crate is extracted, update dependency enforcement and quality/coverage source
  sets together so its functions remain subject to the gates.
- Replace obsolete interactive bash 3.2 qualification in
  `climon2-test-term/src/shell.rs`, affected tests, README and CI matrices.
  Keep non-interactive system-bash uses compatible.

## Gates and unattended permissions

Use existing `cargo xtask` commands; README and `xtask/src/main.rs` describe them.
Every shipped function must satisfy **CRAP <=5**, and mutations must have
**zero survivors**. Preserve append-only historical fixtures, hook-exclusion,
coverage/instrumentation/worker-isolation proofs and actual previous/current
installed-archive compatibility. Include newly shipped source in those gates.

The handoff host was native Intel (`x86_64`). Recheck rather than assume arm64.
Universal `dist` requires arm64, and final archive tests require both native
architectures. Use available CI/appropriate runners; report unavailable gates
instead of substituting an architecture or declaring a complete release pass.

The user explicitly authorized:

- Fresh, isolated Terminal.app/VS Code windows for automated sanity checks.
  Control only test-owned windows with isolated data/profiles; screenshots,
  clipboard access and existing user windows are outside that permission.
  If permissions or reliable targeting are unavailable, report the check blocked
  and keep guards intact.
- Commit **passing increments** and push directly to `main`, preserving unrelated
  user changes. Use the implementing agent's own required commit trailers.

Keep an action log. Clean up only resources owned by your run. Implementation
authorization does not authorize publishing a release or closing decision tickets
before their required work is complete.
