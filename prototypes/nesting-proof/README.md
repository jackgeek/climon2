# PROTOTYPE — nesting proof (#50, cross-ref #44)

Throwaway experiment. Not product code. Not wired into the workspace. Wire schemas and
integration remain #48. Answers the four questions of the nesting/hello experiment handoff
with behavioural tests run against real PTY sessions on macOS.

- Standalone crate (own `[workspace]`); depends on shipped `climon2-core` for the frozen frame
  codec, `HelloReply`, `RuntimeLayout` and the first-release ("legacy") daemon behaviour.
- `nest-daemon` — a PTY daemon (openpty, setsid, TIOCSCTTY, daemon stays the parent of the
  session leader exactly like shipped daemons). `--mode new` advertises nesting state in hello;
  `--mode legacy` uses shipped core code and closes on any unknown post-hello message.
- `nest-client` — `attach`, `hello`, `raw-nest-attach`, `list`.
- `src/graph.rs` — pure admission decision (`admit`), unit-tested.
- `src/proof.rs` — encloser proof; `src/runtime.rs` — lock, enumeration, hello snapshot;
  `src/sys.rs` — macOS kernel queries; `src/wire.rs` — frames (0x30 `NEST_ATTACH` is prototype-only).
- `tests/nesting.rs` — 21 integration tests; `RESULTS.txt` — captured runs.

## Verdicts

| # | Question | Verdict |
|---|----------|---------|
| 1 | Ancestry proof | **PASS** for direct and in-session nesting (env tampering, cross-home, home aliases, in-session relays, orphaned clients, invisible/foreign enclosers fail closed). **BLOCKED** for relays outside the client's process tree (screen/tmux server started elsewhere, ssh to same host) — no local kernel fact distinguishes them from a top-level terminal. `sudo`/`su` only simulated. |
| 2 | Graph identity + atomic admission | **PASS** with target-held in-memory edges advertised in hello + a per-user `flock` held from snapshot until the target ACKs. Introduces new frozen cross-version contracts (user decision). |
| 3 | Legacy participants | **PASS**. |
| 4 | Hello compatibility | **PASS** for new-daemon/old-list and old-daemon/new-list. Old `climon2-client` post-hello against a new daemon not exercised (ADR 0001 forbids cross-version post-hello). |

### Q1 — how a client proves its encloser

Evaluated candidates:

- **Controlling-terminal ownership (chosen primary).** `tcgetsid` on the client's stdin/stdout
  terminal gives the session leader; its parent (`proc_pidinfo`) is the owning daemon. Daemon
  pids are obtained from the kernel (`LOCAL_PEERPID` on each runtime socket), never from hello
  bodies. Works for first-release daemons too (they exec the command in place).
- **Process ancestry (supplementary).** Any daemon pid in the client's ancestor chain. Catches
  in-session relays (`script`, `expect`) that give the client a fresh PTY. Conservative: a GUI
  terminal launched from inside a session would also count as enclosed.
- **Daemon-issued tokens / environment.** Rejected: an environment variable is scrubbed or
  forged trivially. Tests show the proof ignores `CLIMON2_SESSION_*` entirely.

| Case | Result | Test |
|------|--------|------|
| self attach | 125 | `self_attachment_is_rejected_with_125` |
| A in B, then B in A | admitted, then 125 | `acyclic_nesting_is_admitted_and_reverse_attachment_is_rejected` |
| 3-node cycle | 125 | `indirect_cycle_across_three_sessions_is_rejected` |
| cross-home | cycle 125, nesting OK | `cross_home_cycle_is_rejected_and_cross_home_nesting_works` |
| symlink / case-alias homes | same graph, one runtime dir | `home_aliases_resolve_to_the_same_graph` |
| env scrubbed / forged | no effect | `proof_ignores_tampered_environment`, `forged_environment_does_not_block_safe_attachment` |
| `script` relay inside session | 125 (ancestry) | `local_relay_inside_a_session_is_caught_by_process_ancestry` |
| reparented (orphaned) client on session tty | 125 (tty) | `orphaned_client_on_the_session_terminal_is_caught_by_the_tty_proof` |
| encloser daemon not enumerable (stands in for sudo/su/other uid) | 125 fail closed | `enclosing_daemon_outside_the_visible_runtime_directories_fails_closed` |
| screen server started outside, `screen -x` from inside a session | **cycle NOT detected** | `screen_relay_cycle_is_not_detected` (documents the gap) |

Not run: tmux (not installed; screen represents the class), ssh to localhost (Remote Login off;
same class as screen analytically — sshd is not a descendant of any session), real `sudo`
(needs a password). Analytically: a root client of a user's daemon is refused by the shipped
peer-UID check; inside `sudo`/`su` the user's runtime dirs are not enumerated under the new uid,
so the encloser is an unknown daemon, which fails closed via the exe-name test above.

### Q2 — graph and admission

- Graph lives distributed: each **target daemon** keeps in memory one edge set per active
  attachment (the attaching client's enclosers) and advertises them in hello as
  `nesting.incoming`. Edges die with the connection; a daemon crash hangs up its PTY so its
  clients die too. Edges are recorded before the ACK.
- Admission: take a per-user `flock` (`<runtime root>/climon2-<uid>.nesting.lock`, independent of
  home) → hello-snapshot every `climon2-<uid>-*/*.sock` in parallel → walk backwards from the
  enclosers; reaching the target is a cycle → send attach → wait for ACK → release lock.
- Fail closed: any legacy node visited; unresolved source while any daemon is unresponsive; any
  candidate that is an unreachable daemon. Rejected attaches never contact the target.

| Case | Result | Test |
|------|--------|------|
| simultaneous A→B and B→A, 8 rounds | exactly one admitted each round | `simultaneous_cross_attachments_never_both_succeed` |
| same with lock disabled (negative control) | both admitted — race is real | `without_the_lock_simultaneous_cross_attachments_both_succeed` |
| disconnect | edge removed, reverse now admitted | `disconnect_removes_the_edge` |
| control change / rejected attempt | edge kept, target undisturbed | `control_change_keeps_the_edge_and_rejection_disturbs_nothing` |

### Q3 — legacy participants

| Case | Result | Test |
|------|--------|------|
| legacy daemon receives post-hello nesting message | closes, no edge | `legacy_daemon_cannot_take_part_post_hello` |
| new client inside a legacy session attaching to new target | 125 | `nesting_inside_a_legacy_session_fails_closed` |
| new client nesting into a legacy target | 125; top-level attach beside legacy still works | `legacy_target_fails_closed_and_top_level_still_works_beside_legacy` |
| edge hidden inside a legacy daemon | cannot close a cycle (fail closed) | `hidden_legacy_edge_cannot_close_a_cycle` |

### Q4 — hello compatibility

| Case | Result | Test |
|------|--------|------|
| new list, legacy + new daemons | both listed; legacy `state: null` (unknown) | `new_list_shows_unknown_state_for_legacy_and_state_for_new` |
| shipped first-release `climon2-cli list --json`, new daemon with extra optional fields | listed (core `HelloReply` ignores unknown fields) | `shipped_first_release_list_shows_new_daemons_with_optional_fields` |

## Open decisions / blockers for the user

1. **Relay scope (BLOCKED).** Out-of-tree relays (screen/tmux servers, ssh to same host) cannot
   be detected; failing closed for them would refuse all top-level attaches from such
   terminals. Decide: guarantee covers direct/in-session nesting only, or require another
   mechanism (none found locally).
2. **Lock + enumeration as frozen contracts, or a per-user coordinator process.** The prototype
   freezes a per-user, home-independent lock path and the `climon2-<uid>-*` runtime-dir naming
   across versions. A coordinator is the alternative (new decision).
3. **Freeze hello `nesting.incoming`** (optional field forever) — exposes other sessions' GUIDs
   to any same-uid prober.
4. **Fail closed when any daemon is unresponsive** while the source is unresolved (one hung daemon
   blocks nesting; probe cost up to 2 s per attach).
5. **Cross-uid / root handling and the exe-name heuristic** (`climon2-daemon` name identifies an
   unreachable encloser). Real `sudo`/`su` untested.
6. **Ancestry false positives** (GUI terminal launched from inside a session counts as nested).
7. PID-reuse protection (start-time checks) is skipped in the prototype.

## How to run

```sh
# from the repo root: shipped CLI used by the old-list probe
cargo build -j1 -p climon2-cli
cd prototypes/nesting-proof
cargo test -j1 -- --test-threads=1
```

macOS only (`proc_pidinfo`, `LOCAL_PEERPID`). Needs `/usr/bin/script` and `screen`. Tests create
runtime roots under `<repo>/rt/` and work dirs under `work/` (both ignored).

## Commands and results

- `cargo test -j1 -- --test-threads=1` ×3 consecutive: 9 unit + 21 integration passed each run
  (~10 s). See `RESULTS.txt`.
- Mutation checks (source temporarily edited, then restored):
  - drop tty source → orphaned-client test fails;
  - drop ancestry source → script-relay test fails;
  - treat legacy nodes as edge-free → both legacy-cycle tests fail;
  - ignore unreachable enclosers → unreachable-encloser test fails;
  - disable lock → covered by the negative-control test (both admitted).
- Tests were written first; stub binaries failed (daemon ready timeout) before implementation.

## Action log

1. Read handoff, AGENTS.md, domain docs, ADRs 0001/0002/0003/0005, #41/#42/#44/#47/#50 comments
   and shipped protocol/runtime/peer/list code — to stay inside frozen contracts.
2. Worktree `prototype/nesting-proof-50` from `origin/main` 79202a5 at a short path — socket
   paths must stay under the 104-byte `sun_path` limit.
3. Standalone crate reusing `climon2-core` — legacy mode must be the real first-release code.
4. Tests first against stubs (red), then implementation (green).
5. Added home-alias assertion of a single runtime dir — APFS canonicalisation merges case aliases.
6. Added orphaned-client test — only the tty proof catches it; proves both sources are needed.
7. Screen relay test: run server outside the session and verify `screen -ls` "Attached"
   — `screen -x` with `</dev/tty` silently exits, so it runs in the plan foreground.
8. Lock negative control — demonstrate the race exists, so the lock test is meaningful.
9. Unreachable-encloser test — stand-in for sudo/su since `sudo -n` needs a password.
10. Mutation checks, then 3 full runs captured in `RESULTS.txt` — race stability.
