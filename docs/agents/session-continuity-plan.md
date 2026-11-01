# Session continuity: implementation plan

This plan is self-contained. Every product requirement you need is stated here;
do not look for requirements in GitHub issues, earlier handoff notes or prototype
branches. Where this plan says "choose", the choice is yours; everything else is a
settled decision. Code references are to this repository at the time of writing
(`main` after `e0d9b5d`); re-verify line numbers before editing.

## 0. Ground rules

- Read `AGENTS.md` and follow it. Invoke the `ub-mode` skill before planning or
  writing code and follow it: red-green TDD for every behaviour change, smallest
  edit, pure business logic behind checked boundaries, keep an action log and
  report it at the end.
- A feature is incomplete until automated tests cover it.
- Every function in shipped source must have **CRAP <= 5** (crapper; `?` counts as
  a branch) and mutation testing must leave **zero surviving mutants**. No
  exemptions. Any new shipped crate must be added to the gates (section 7).
- The user's uncommitted `CLAUDE.md` change is unrelated: never stage, commit,
  revert or stash it. Check `git status` before every commit.
- Authorised: commit **passing** increments and push directly to `main`, with
  your own required commit trailers. Small ordered commits; each message says
  why, what changed and how it was proven.
- Authorised: open fresh, isolated Terminal.app / VS Code windows for automated
  native sanity checks (section 9). Only control windows your run created, with
  isolated data directories. Screenshots, clipboard access and the user's existing
  windows are **not** authorised.
- Not authorised: publishing a release, creating release tags, or closing GitHub
  issues.
- The user may be unavailable. Ordinary design choices do not need approval. If
  something is genuinely blocked (missing permission, unavailable architecture),
  record it, keep the guard in place, and continue with independent work.
- Clean up only resources your run created (temporary homes, sockets, windows,
  processes, worktrees).

## 1. Product specification

### 1.1 Commands, usage and results

New usage string (replace `USAGE` in `crates/climon2-core/src/cli.rs`):

```text
usage: climon2 exec <command> [args...] | attach <session-id> | kill <session-id> | list [--json]
```

| Command | Behaviour |
|---|---|
| `climon2 exec <command> [args...]` | Unchanged start-attached behaviour, plus configuration validation before startup, continuity (detach/reattach), and the new client keys. No start-detached option. |
| `climon2 attach <session-id>` | Attach to one live session and take control immediately. Requires stdin **and** stdout to be TTYs. |
| `climon2 kill <session-id>` | Deliberately terminate one session. No TTY needed. Silent on success. |
| `climon2 list [--json]` | Existing listing plus VERSION, STATE and CLIENTS (section 1.11). |

Exit statuses (keep `crates/climon2-core/src/exit.rs` constants):

- `0`: explicit detach confirmed by the daemon; `kill` confirmed termination and cleanup.
- Command's own status: the session's command finished while this client was
  attached (controlling or not). Signal-derived statuses stay `128 + signal`.
- `2`: missing/malformed arguments or selector (`UsageError`).
- `125`: runtime or configuration failure: unknown or ambiguous selector,
  unsupported older session, missing matching client binary, non-TTY attach,
  self-attach, configuration error, attach refused because the session is
  terminating, unexpected daemon connection loss, unconfirmed detach, failed or
  unconfirmed kill.
- `128 + signal`: the client itself was terminated by a handled signal
  (existing behaviour). Existing `TERMINAL_GONE = 129` on terminal input EOF stays.
- Restore the local terminal before writing any diagnostic. Keep the existing
  one-line `climon2: error: <message>` contract and the ` (logs: <path>)` suffix
  rules (`crates/climon2-cli/src/lib.rs`, `crates/climon2-client/src/lib.rs`).
- No success banner on attach; nothing on stdout for attach/detach/kill.
- Explicit detach writes one short stderr line after restoring the terminal, e.g.
  `climon2: detached from session <full-id>; reattach with: climon2 attach <full-id>`.
- Unexpected connection loss is never reported as a detach.

### 1.2 Session selectors (attach and kill)

- Accept a full GUID or a unique prefix of **at least 8** hexadecimal digits,
  case-insensitive, with or without hyphens. Normalise by removing hyphens and
  lowercasing; then require 8..=32 hex digits. Anything else (including omitted)
  is a usage error (2).
- Match against **live** sessions in the selected climon2 home only (sessions whose
  hello probe succeeds, as `list` already determines). Sessions from versions that
  do not support the operation stay in the candidate set (never drop them to make
  an ambiguous prefix unique).
- No match: 125. More than one match: 125 and the message lists every matching
  full ID. No names, pickers, "latest" selection, or retargeting if the selected
  session disappears; a target that disappears or changes identity between
  selection and dispatch fails with 125.

### 1.3 User configuration: `<climon2 home>/config.jsonc`

```jsonc
{
  "client": {
    "escape_prefix": "Ctrl+]",     // null disables escape interception
    "take_control_key": "Ctrl+T"   // null disables keyboard takeover
  },
  "scrollback": {
    "max_rows": 10000
  },
}
```

- JSON with `//` and `/* */` comments and trailing commas. A missing file, a
  missing object or a missing field means the default. `null` disables a binding.
- Key strings (exact spelling): `Ctrl+A`..`Ctrl+Z` (bytes 0x01..0x1A), `Ctrl+[`
  (0x1B), `Ctrl+\` (0x1C), `Ctrl+]` (0x1D), `Ctrl+^` (0x1E), `Ctrl+_` (0x1F),
  `Ctrl+Space` (0x00). Nothing else: no printable keys, Alt, function keys or
  multi-key sequences.
- `take_control_key` may not be `Ctrl+[` (Escape, 0x1B). `escape_prefix` may be.
- If both bindings are enabled and resolve to the same byte: configuration error.
- `scrollback.max_rows`: integer 0..=100000, default 10000. 0 disables retained
  history only; the current screen is still restored. (`scrollback.max_bytes` is
  **not** a setting; like any unknown field it is ignored.)
- Errors (all exit 125, before raw mode and before any session side effect):
  unreadable existing file, malformed JSONC, duplicate keys in a known object,
  wrong type for a known field, invalid key string, binding conflict, out-of-range
  `max_rows`. Unknown fields are ignored.
- Scope of validation by operation:
  - `exec`: validate `client` **and** `scrollback` before starting the daemon.
  - `attach`: validate `client` only (a malformed file still fails; an invalid
    unused `scrollback` value does not).
  - `list`, `kill`: do not read the file.
- Read once per operation. The initial client of `exec` must use exactly the
  snapshot validated before startup (no re-read). A running daemon's retention
  never changes after startup; attaching never changes it.
- No environment-variable override, config commands, auto-creation, rewriting or
  live reload.

### 1.4 Client keys: escape, detach, passthrough, paste, takeover

Implement as a pure input filter in the client.

- Default escape prefix Ctrl+] (0x1D). Prefix then lowercase `d`: detach this
  client. Prefix then prefix: send **one** literal prefix byte to the session
  immediately. Prefix then any other byte: forward both bytes unchanged. A lone
  prefix waits indefinitely for the next byte (no timeout); output keeps flowing.
- Ctrl+C, Ctrl+D etc. are forwarded unchanged; only configured bindings are
  intercepted.
- Bracketed paste: input between `ESC [ 200 ~` and `ESC [ 201 ~` is paste. Inside
  paste, no binding is interpreted. Markers may be split across reads; buffer
  partial markers. Unmarked pastes are indistinguishable from typing (document).
- Take-control key (default Ctrl+T, 0x14): intercepted **only while
  non-controlling** and outside paste; consumed as a take-control request, never
  sent to the command. While controlling it is an ordinary byte (unless it is also
  the escape prefix, which is impossible by validation).
- Non-controlling client: the escape prefix and detach still work; all other
  application input (including paste content) is discarded.
- With escape interception disabled (`null`) there is no in-band detach; closing
  that client is an unexpected departure (section 1.7).

### 1.5 Control, readiness and stale input

- The daemon is the sole authority over attachments, control and session size.
  Whenever at least one client is attached exactly one is the controller.
- Explicit attach (including the initial `exec` client) takes control immediately.
  The previous controller stays attached as non-controlling.
- Attach and take-control requests are serialised in daemon acceptance order; each
  accepted request takes control; last accepted wins. No approval prompt, lock or
  inactivity requirement.
- A take-control request from the current controller is a no-op (no new control
  period, no hand-back history entry, no screen reset).
- Controller departure: hand control back automatically to the most recently
  controlling client that is still attached. Each actual grant makes the recipient
  the most recent controller.
- **Control period**: every grant starts a new period (monotonic `u64`). The new
  controller's application input stays blocked until it has received and written
  the complete restoration for that period and confirmed readiness. Input typed
  or queued before readiness is discarded (never replayed).
- When control transfers, input from the old controller that has not yet been
  written to the PTY is discarded. Late input from an earlier period is ignored,
  even if the same client later regains control. Bytes already written to the PTY
  cannot be recalled.
- Non-controlling clients receive **no** session content: on revocation the
  daemon stops forwarding output to that client, and the client shows a placeholder.
  Wire ordering must guarantee stale queued output cannot overwrite the
  placeholder after the revocation is processed (send the revoke message after any
  queued output on the same connection).
- Placeholder (drawn by the client; preserve visible outer content per 1.8): a
  blank screen whose first line is `This session is being viewed elsewhere.` and
  whose second line is `Press <key> to take control` with the configured key
  string (e.g. `Ctrl+T`). Omit the second line when takeover is disabled.
- A rejected takeover leaves the controller unchanged. No half-accepted ownership.
- Connection cap: keep the existing 64 simultaneous daemon connections
  (clients + probes). Exhaustion never evicts a client or changes control; an
  attachment that cannot be admitted fails explicitly (125).

### 1.6 Sizing

- Only the controller's size drives the PTY and the daemon's virtual terminal.
  Non-controller resizes are remembered only.
- On grant (attach, takeover, hand-back) apply the new controller's latest valid
  size before producing its restoration.
- Maximum session geometry **500 columns x 200 rows**; larger host sizes are
  clamped. Invalid initial size (zero) falls back to 80x24 (existing
  `TerminalSize::initial`); later zero-sized updates are ignored (existing
  `TerminalSize::resize`).
- Intermediate resizes may be coalesced. If the controller resizes before it has
  confirmed readiness, issue a fresh restoration at the new size (new period).
  After readiness, resizes are applied normally (the application redraws).
- After an explicit detach leaves zero clients, keep the last controller's size.

### 1.7 Lifetime without clients and deliberate termination

- One daemon-owned leave policy evaluates every departure:
  - Another attached client remains (controlling or not): the session continues;
    apply hand-back if the controller left.
  - The final client leaves by an **accepted explicit detach**: the session keeps
    running indefinitely (no idle timeout) until the command exits or it is killed.
  - The final client leaves **unexpectedly** (terminal loss, client crash, socket
    closed without an accepted detach, queue overflow, restore deadline expiry):
    start termination immediately. No grace for reconnection; no cancellation.
  - An accepted detach is not sticky: after reattachment, a later unexpected loss
    of the final client starts termination again.
  - Probes, kill-mode connections and rejected or not-yet-accepted connections are
    not attachments; their departure never triggers this policy.
- Explicit detach boundary: departure is intentional only after the daemon accepts
  a valid detach request. Loss before acceptance is unexpected; loss after
  acceptance stays intentional even if the confirmation is lost. The client reports
  success only after receiving the confirmation; otherwise 125 ("detach not
  confirmed").
- While detached the daemon keeps reading PTY output and updating its virtual
  terminal and bounded history. Absence of clients never blocks the command's
  output.
- Command exit ends the session: keep the existing bounded drain
  (`PTY_DRAIN_IDLE` 100 ms / `PTY_DRAIN_CAP` 1 s), deliver `Exit{code}` to every
  attached client (controlling or not), remove metadata and socket, release state.
  No completed-session state, no later attachment.
- **Initial attachment obligation**: if no client has attached within 5 seconds of
  daemon readiness, terminate the session (failed startup, not detach). Probes and
  rejected connections neither satisfy nor extend this deadline. A command that
  exits first follows normal completion.
- Command completion, attach, detach and kill are serialised by the daemon. Once
  shutdown begins, refuse new attachments (125 at the client) and do not let
  attach/detach cancel shutdown.
- `climon2 kill <selector>`: single target, no `--all`, force, signal choice or
  prompt; works without TTY and whether or not clients are attached. The target
  daemon owns shutdown; never signal a PID read from metadata. Kill and unexpected
  final loss both use the existing shutdown sequence: hang up the PTY, existing
  5-second grace (`HANGUP_GRACE`), then forcibly terminate remaining managed jobs
  (`Pty::leave` / `Pty::finish` in `crates/climon2-daemon/src/pty.rs`). Preserve
  the existing process-identity/ownership safeguards.
- Repeated kills join the existing shutdown and do not restart the grace. An
  attach accepted before the kill is terminated by it; an attach after shutdown
  begins fails.
- Kill result: silent `0` only after the daemon confirms the command terminated
  **and** cleanup (metadata and socket removal) finished. One monotonic **10-second
  overall deadline** in the CLI covers selection, probing, helper startup, request
  and completion waiting. On expiry: 125 "termination of session <id> was not
  confirmed" (do not claim it is still running); an accepted shutdown continues.
  The CLI may stop and reap only the helper process it spawned.
- Clients attached during kill receive the command's actual (signal-derived)
  status if delivered; connection loss before a terminal result is 125.
- Killing the enclosing session is allowed without a prompt.

### 1.8 Restoration, history, title and hyperlinks

The daemon owns one virtual terminal (Alacritty, section 3.2) per session.
Clients keep no emulator and no hidden transcript.

On **every grant** (initial exec attach, explicit attach, takeover, hand-back,
re-grant after a pre-readiness resize) the daemon sends the new controller a
restoration, then live output:

0. **Prologue** (host may hold modes from an earlier application): end any open
   synchronized update (`CSI ? 2026 l`), reset margins (`CSI r`), origin off
   (`CSI ? 6 l`), insert off (`CSI 4 l`), line-feed/new-line off (`CSI 20 l`),
   autowrap on (`CSI ? 7 h`), `CSI 0 m`. The steps below rely on these.
1. **Preserve visible outer content**: move the host cursor to the bottom row
   (`CSI <rows> ; 1 H`, using the controller's reported rows) and emit `rows`
   line feeds so the currently visible host screen scrolls into native history.
   Never send `CSI 2 J`/`CSI 3 J`/`ESC c` or anything that clears native history.
2. **Truncation notice**: if the daemon knows retained history was evicted (see
   3.2) and history is non-empty, emit the line
   `[climon2: older session scrollback was discarded]` (default rendition),
   followed by CR LF. This is presentation text, not part of retained history; it
   is emitted on every such restoration.
3. **Normal history and screen**: render every retained history row
   (oldest first) and then every normal-screen row from parsed cell state: text,
   wide characters, combining marks, and SGR rendition (bold, dim, italic,
   underline styles, inverse, hidden, strikeout, fg/bg/underline colours). Hard
   line ends as CR LF; soft-wrapped rows are written full-width without CR LF so
   the host wraps them itself (if the host is wider than the session because of
   the 500-column clamp, use CR LF for every row). Never replay raw historical
   output, clears, bells, queries, clipboard operations or other side effects.
   Hyperlinks become plain text.
4. **Normal cursor**: establish the normal screen's saved cursor (position and pen,
   then `ESC 7`), then the current cursor position and pen. If the cursor has a
   pending wrap (`Cursor::input_needs_wrap`), position at the last column and
   re-print that cell's character with its rendition so the host also enters the
   pending-wrap state (CUP alone clears it).
5. **Alternate screen (eager)**: if the application is on the alternate screen,
   emit `CSI ? 1049 h`, render the alternate screen rows (no history), establish
   its saved cursor and current cursor/pen (including pending wrap, as in step 4).
   When the application later leaves the
   alternate screen the host shows the correct normal content.
6. **Modes**: emit DECSET/DECRST to make the host match the virtual terminal's
   modes that affect display or input: cursor visibility (25), application cursor
   keys (1), application keypad (`ESC =`/`ESC >`), bracketed paste (2004), mouse
   reporting modes (1000/1002/1003) and encodings (1006/1005), focus reporting
   (1004), alternate scroll (1007), autowrap (7), origin (6), insert (`CSI 4 h/l`),
   line feed/new line (`CSI 20 h/l`), cursor style (DECSCUSR from
   `Term::cursor_style`).
7. **Title**: emit `OSC 2 ; <title> ESC \` with the application's current title,
   or the default title when none is set (section 1.8.1).
8. Mark the end of restoration (protocol, 3.4). Live output follows in order.

Required: when the application is in the alternate screen, the underlying
normal screen and history are still restored (step 3 before step 5).
Not restored (document as limitations): scroll region, tab stops, character sets,
palette changes (OSC 4/10/11), keyboard-protocol modes, hyperlinks.

Repeated attachment repeats history the host may already hold: native duplicates
are accepted. There is no incremental catch-up and no per-client delivery
tracking; every grant resends all retained history.

Live output that clears/resets the host (an application running `clear`) keeps its
ordinary meaning.

#### 1.8.1 Titles

- Default title: `climon2 exec <command> <args...>` joined by single spaces from
  the daemon's application argv (lossy UTF-8 for non-UTF-8 arguments), e.g.
  `climon2 exec bash` or `climon2 exec vim notes.txt`.
- Application titles (OSC 0/2, title stack push/pop) replace it; a title reset
  returns to the default.
- When emitting any title, preserve ordinary Unicode and percent-encode, as
  uppercase `%XX` of each UTF-8 byte, every C0 control (U+0000..U+001F), DEL
  (U+007F) and C1 control (U+0080..U+009F, i.e. `%C2%80`..`%C2%9F`). Encode
  nothing else.

### 1.9 Live output and query authority

- The daemon reads the PTY continuously; every byte updates the virtual terminal.
  The same bytes, filtered, are forwarded to the controller only (when one exists
  and has a restoration queued).
- The daemon is the sole answerer of the queries the engine answers. Write the
  engine's replies (`Event::PtyWrite`) to the PTY whether or not clients are
  attached, and **remove those queries from the live stream** so the host never
  answers them too:
  - `CSI c`, `CSI 0 c` (DA1), `CSI > c`, `CSI > 0 c` (DA2), `ESC Z` (DECID)
  - `CSI 5 n`, `CSI 6 n` (DSR)
  - `CSI Ps $ p` and `CSI ? Ps $ p` (DECRQM)
  - `CSI 18 t` (text area size in characters)
- Input is legacy-only: also remove keyboard-protocol negotiation from the live
  stream: `CSI ? u`, `CSI > … u`, `CSI < … u`, `CSI = … u` (kitty keyboard) and
  `CSI > … m` (xterm modifyOtherKeys). Configure the engine with
  `kitty_keyboard: false`.
- Queries the engine does not answer (colour queries OSC 4/10/11/12 with `?`,
  `CSI 14 t`, `CSI ? 6 n`, OSC 52 read, DCS requests) pass through to the host;
  ignore the corresponding engine events (`ColorRequest`, `TextAreaSizeRequest`,
  `ClipboardLoad`) so nothing is answered twice. While detached they go
  unanswered.
- Ignore engine `Title`/`Bell`/`ClipboardStore` events for side effects (track the
  title only); live bytes already carried them to the host.
- Control strings (OSC, DCS, SOS, PM, APC): a filter in front of the parser holds
  each one until its terminator; if its body exceeds **4096 bytes** it is dropped
  from both the engine and live output, logged without content, and the session
  continues. Apply the same rule to CSI sequences longer than 4096 bytes.
- Hyperlinks (OSC 8) never fail or terminate a session. Live OSC 8 within the cap
  passes through; restoration renders linked text as plain text.

### 1.10 Internal budgets (not user configurable)

| Budget | Value |
|---|---|
| Control-string / CSI cap | 4096 bytes body; oversized dropped and logged |
| Geometry | max 500x200; larger clamped; zero rejected (80x24 initial fallback) |
| Engine step | feed the engine at most 64 bytes per step, at token boundaries where possible |
| Synchronized output (`CSI ? 2026 h/l`) | use the engine's built-in handling; when its 150 ms timeout passes call `Processor::stop_sync` (this may process up to the engine's 2 MiB sync buffer in one call: an accepted exception to the 64-byte step); hold any restoration snapshot until no synchronized update is pending (max ~150 ms). Live forwarding: the filter tracks whether the controller's host has an open update (`CSI ? 2026 h` forwarded without its `l`); on engine timeout, revocation, release or restoration emit a synthetic `CSI ? 2026 l` so the host never stays buffered |
| Per-attachment outbound queue | 16 MiB; overflow disconnects that attachment as an unexpected departure |
| Restoration delivery | no write progress for 5 s, or not fully written within `30 s + restore_bytes / 4 MiB/s`: disconnect as unexpected departure |
| Initial attachment | 5 s after readiness |
| Kill operation | 10 s overall (CLI) |
| Connections | 64 (existing `MAX_CONNECTIONS`) |
| Startup envelope | 4096 bytes |

### 1.11 Discovery, hello extensions and listing

Hello stays frozen (section 1.15). The new daemon adds **optional** reply fields:

```json
{"session_id":"…","climon2_version":"…","protocol":1,
 "capabilities":["attach","kill","state-v1","client-startup-v1"],
 "state":{"lifecycle":"attached","attachments":2,"controlled":true}}
```

- `capabilities`: array of strings. Unknown tokens ignored. Never infer support
  from versions or protocol numbers. A malformed advertisement grants nothing and
  is logged, but does not invalidate the mandatory hello or mark the session stale.
  Do **not** advertise `nesting-v1`.
- Public attach requires `attach` + `client-startup-v1`; public kill requires
  `kill` + `client-startup-v1`; showing state requires `state-v1`.
- `state` is one consistent point-in-time snapshot:
  - `starting`: awaiting initial attach; 0 attachments; `controlled:false`.
  - `attached`: attachments > 0; `controlled:true`.
  - `detached`: after accepted explicit detach; 0 attachments; `controlled:false`.
  - `terminating`: shutdown under way; attachments >= 0; `controlled` only if
    attachments > 0.
  `attachments` counts accepted attachments only.
- `list --json`: the existing array of metadata objects, each with an added
  `"state"` key holding that object or `null` (no `state-v1`, or missing,
  malformed or inconsistent state; log malformed data). Do not change metadata.
- Human `list`: keep SESSION, CREATED, COMMAND and ARGS, safe quoting/truncation
  and newest-first order; add VERSION (metadata `climon2_version`), STATE
  (lifecycle or `unknown`) and CLIENTS (attachment count including `0`, or `-`).
  Suggested order: SESSION, CREATED, VERSION, STATE, CLIENTS, COMMAND, ARGS.
- Listing never requires the matching client binary.

### 1.12 Version-matched dispatch and the client startup envelope

The CLI never talks post-hello to a daemon. For attach and kill it:

1. Selects the target (1.2) and checks the required capabilities.
2. Checks the matching client exists at
   `<executable home>/<target climon2_version>/climon2-client`
   (`ExecutableLayout::client_invocation` in `crates/climon2-core/src/layout.rs`);
   otherwise 125 naming the version. Never use a different version's client, never
   install anything.
3. Attach only: requires stdin/stdout TTYs, the self-attach check (1.13) and a
   validated client-settings snapshot.
4. Invokes the frozen argv `climon2-client attach --metadata <absolute path>`
   with the environment variable `CLIMON2_CLIENT_STARTUP` set to a schema-1 JSON
   envelope (always overwriting any inherited value):

```json
{"schema":1,"operation":"attach","settings":{"escape_prefix":29,"take_control_key":20}}
{"schema":1,"operation":"kill"}
```

Envelope rules (client side, before raw mode or any daemon request; all failures 125):

- UTF-8 JSON object, at most 4096 bytes. `schema` must be `1`; `operation` must be
  exactly `attach` or `kill`.
- `attach` requires `settings` with both fields present; each is an integer
  control byte (0..=31) or `null`. Revalidate the vocabulary, the take-control
  `27` exclusion and the conflict rule. Never fill missing fields from defaults.
- `kill` must not carry `settings`.
- Duplicate known fields, missing or wrong-typed known fields, malformed JSON,
  invalid UTF-8 and >4096 bytes are errors. Unknown optional fields are ignored.
- A present valid envelope is used exactly with no config read. A present invalid
  envelope never falls back. An **absent** envelope means attach, and the client
  loads and validates its own client settings once (generic callers of the frozen
  argv keep working).
- The CLI removes `CLIMON2_CLIENT_STARTUP` from the daemon's and application's
  environment.
- `exec` passes the same envelope (operation `attach`, the pre-startup snapshot)
  to the client it `exec`s into.
- The client rechecks session identity, its own version/protocol and the current
  advertised capabilities on its own hello before sending an operation request.
- Kill mode never attaches, takes control, enters raw mode, requires a TTY or
  sends application input; its connection is not an attachment.

Retention handoff: the CLI passes the validated value to the daemon through the
existing private daemon argv as a **required** option
`--scrollback-max-rows <0..=100000>` (before `--`). The daemon revalidates the
range and fails before starting the application on invalid or missing input.

### 1.13 Self-attachment

- If the invoking environment's `CLIMON2_SESSION_ID` (the session enclosing the
  caller) equals the selected target, refuse with 125 and an explicit diagnostic
  before dispatch: no control change, no disruption.
- Do this check in the **CLI** attach path, as a preflight before dispatch (one
  place for the decision). Note: today only the daemon/application receive the
  projected session environment (`env_clear()` + projected values in
  `crates/climon2-cli/src/exec.rs`); the client that `exec` replaces itself with
  inherits the CLI's own environment, so its `CLIMON2_SESSION_ID` is the
  *enclosing* session's (if any). Keep that contract and test it.
- If the enclosing session cannot be identified, do not refuse for that reason.
  No ancestry graph, indirect-cycle detection, locks or nesting advertisement;
  other cycles are the user's responsibility (document this).

### 1.14 Out of scope

The monitor; global/indirect cycle detection; extended keyboard protocols and
application-keypad fidelity (ordinary keys must work); crash/reboot recovery;
completed-session persistence; pixel-exact native rendering; exhaustive
re-testing of Alacritty behaviour; iTerm2; start-detached; session names.

### 1.15 Frozen contracts (must not change)

- Launcher: reads one line from `<home>/current`, runs
  `<home>/<version>/climon2-cli` with unchanged argv; launcher stays dependency-free.
- Client argv: exactly `climon2-client attach --metadata <absolute path>`.
- Framing: big-endian `u32` length (covering the type byte and body), then one type
  byte, then body; length 1..=1,048,576. Data frames (input/output) <= 65,536.
- Hello: type `0x01`; request `{"protocol":1}`; reply with mandatory
  `session_id`, `climon2_version`, `protocol`; 2-second absolute deadline. Optional
  reply fields may be added; readers ignore unknown fields.
- Metadata: `<home>/sessions/<guid>.pty`, write-once, eleven core fields
  (`format_version`, `session_id`, `socket_path`, `daemon_pid`,
  `daemon_start_time`, `command`, `args`, `cwd`, `depth`, `created_at`,
  `climon2_version`); readers accept `format_version >= 1` and ignore unknown
  fields. Never write dynamic state into metadata. Existing goldens under
  `crates/climon2-core/tests/goldens` are append-only.
- Messages after hello are version-private (same-version client and daemon only);
  you may add new type codes and change post-hello bodies freely.
- Older releases are not retrofitted; unsupported operations against them fail
  with 125 without disturbing the session.

## 2. Current code map

Shipped crates (all synchronous; no async runtime):

- `climon2-core`: `cli.rs` (`CliArgs::{Exec, List, Help, Version}`, `USAGE`),
  `exit.rs`, `identity.rs` (`SessionId`, `VersionName`, …), `json.rs`
  (`from_object`), `layout.rs` (`HomeLayout`, `ExecutableLayout::client_invocation`,
  `RuntimeLayout`), `metadata.rs` (`SessionMetadata`), `protocol.rs` (constants,
  `Frame`, `read_frame`/`write_frame`, frame-type validation at the
  `0x01 | 0x10..=0x12 | 0x20..=0x22 | 0x7f` match, `TerminalSize`,
  `ConnectionState`, `HelloReply`, `ClientMessage`, `ServerMessage`),
  `session.rs` (`SessionEnvironment::for_session` injects `CLIMON2_SESSION_ID`,
  `CLIMON2_SESSION_DEPTH`, default `TERM`), `diagnostics.rs`.
- `climon2-cli`: `exec.rs` (TTY preflight, daemon spawn with private argv
  `--session-id --home --socket --version --cli-pid --rows --cols --cwd [--log]
  --readiness-fd -- <command> <args>`, readiness wait, then `CommandExt::exec`
  into the client), `list.rs` (`discover` scans metadata and probes hello in
  scoped threads; stale cleanup via process identity; `table`; JSON).
- `climon2-daemon`: `args.rs` (private argv parser), `startup.rs`
  (prepare/publish/cleanup), `pty.rs` (`Pty::spawn/resize/write/read_output/
  try_wait/leave/finish`), `connection.rs` (per-connection framed I/O and queue),
  `session.rs` (single-threaded loop, `TICK` 2 ms, `OUTPUT_CHUNK` 32 KiB,
  `MAX_CONNECTIONS` 64). Today: one controller only; a second `Attach` is rejected;
  the PTY is read only when the controller can accept output (backpressure); output
  goes only to the controller; controller departure hangs up the session.
- `climon2-client`: `lib.rs` (argv, metadata, logs, hello validation, attach,
  raw mode), `transport.rs` (framed transport, hello/attach), `bridge.rs`
  (`poll` loop over socket/stdin/stdout, SIGWINCH -> `Resize`, signals ->
  `128+signal`, stdin EOF -> 129).
- `climon2-platform`: PTY allocation, processes, signals, termios
  (`RawModeGuard`), sockets, peer credentials, logging, presentation.
- `climon2-launcher`, `climon2-install`: unaffected except docs/tests if needed.

Test-only crates: `climon2-test-term` (real PTY + `alacritty_terminal` 0.26.0;
`Terminal::spawn/send/screen/wait_for_text/wait_for_bytes/resize/wait_exit`;
automatically writes `Event::PtyWrite` replies back; `shell.rs` bash profiles),
`climon2-fixtures` (`climon2-fixture`, `climon2-tui-fixture`,
`climon2-cli-fixture`, `climon2-daemon-fixture`), `climon2-e2e`
(`tests/packaged.rs`, `tests/support/mod.rs` with `Package`).

Gates: `xtask/src/quality.rs` (`SHIPPED` source list, CRAP threshold "5",
crapper + mutator pins, changed-source selection via `CLIMON2_MUTATION_BASE`),
`xtask/src/release.rs` (`PACKAGES`), `xtask/src/dist.rs`, `xtask/src/fixtures.rs`.
CI: `.github/workflows/check.yml`, `release.yml`.

## 3. Design

### 3.1 Crates and dependency direction

- Add shipped crate **`crates/climon2-vt`**: the only crate that depends on
  `alacritty_terminal = { version = "0.26.0", default-features = false }` (crates.io,
  same version already locked for the test harness; no git dependency). It exposes
  climon2-owned types only (no Alacritty types in its public API). Platform-neutral:
  no I/O, sockets or PTYs.
- `climon2-daemon` depends on `climon2-vt`. No other shipped crate does.
- Pure, shared contracts go in `climon2-core`: config parsing (`config.rs`), key
  vocabulary, selector parsing/matching (`selector.rs`), startup envelope
  (`startup_envelope.rs` or similar), hello extensions/capabilities/state, title
  encoding helper if shared, new protocol messages.
- Shipped crates must not depend on test-only crates. Add `crates/climon2-vt/src`
  to `SHIPPED` in `xtask/src/quality.rs` in the same commit that creates the crate,
  and make sure coverage, crapper and mutator see it (re-run
  `cargo xtask tooling-probe` if selection logic is touched). The daemon binary
  packaging is unchanged (`climon2-vt` is a library).

### 3.2 The virtual terminal adapter (`climon2-vt`)

Suggested public surface (adapt as needed, keep it small):

```rust
pub struct Geometry { pub cols: u16, pub rows: u16 }          // already clamped
pub struct VtConfig { pub max_history_rows: u32, pub default_title: String }
pub struct Vt { /* Term, Processor, filter, event queue, title, eviction flag */ }

impl Vt {
    pub fn new(geometry: Geometry, config: VtConfig) -> Self;
    /// Feed PTY output. Returns bytes to forward live and replies to write to the PTY.
    pub fn feed(&mut self, bytes: &[u8]) -> Fed;               // Fed { live: Vec<u8>, replies: Vec<u8>, dropped: usize }
    pub fn resize(&mut self, geometry: Geometry);
    pub fn sync_pending(&self) -> bool;                         // synchronized update in progress
    pub fn tick(&mut self, now: Instant) -> Fed;                // ends expired synchronized updates; returns
                                                                // any replies/title changes plus synthetic ESU
    pub fn at_boundary(&self) -> bool;                          // no partial token buffered
    pub fn restoration(&mut self, host: Geometry) -> Vec<u8>;   // section 1.8 steps 1-7
    pub fn release(&self) -> Vec<u8>;                           // host reset bytes (3.5)
}
```

Implementation notes (Alacritty 0.26 public API):

- `Term::new(config, &dimensions, listener)` with
  `term::Config { scrolling_history: max_rows, kitty_keyboard: false,
  osc52: Osc52::Disabled, ..Default::default() }`. Implement `Dimensions` with a
  tiny struct (or use the public `alacritty_terminal::term::test::TermSize`).
- Parser: `alacritty_terminal::vte::ansi::Processor` (`advance(&mut term, bytes)`;
  `sync_timeout().sync_timeout()` returns the pending deadline; `stop_sync(&mut term)`).
- Events: `EventListener::send_event(&self, Event)` takes `&self`; use a listener
  holding `Rc<RefCell<Vec<Event>>>` shared with the adapter (the daemon is single
  threaded). Drain after each feed: `PtyWrite(text)` -> replies; `Title(t)` /
  `ResetTitle` -> tracked title; ignore everything else.
- Active grid: `term.grid()` (`Grid<Cell>`; `Dimensions::{history_size,
  screen_lines, columns, topmost_line, bottommost_line}`; indexing by `Line`/`Column`;
  `grid.cursor` and `grid.saved_cursor` have public `point` and `template`).
  Cell data: `c`, `fg`, `bg`, `flags` (`WRAPLINE`, `WIDE_CHAR`,
  `WIDE_CHAR_SPACER`, `LEADING_WIDE_CHAR_SPACER`, `BOLD`, `DIM`, `ITALIC`,
  underline variants, `INVERSE`, `HIDDEN`, `STRIKEOUT`), `zerowidth()`,
  `underline_color()`. Modes: `term.mode()` (`TermMode`). Cursor style:
  `term.cursor_style()`.
- **Reading the normal screen while the alternate screen is active.** The inactive
  grid is private. Use this sequence (single-threaded, inside `restoration`):
  1. `let alt = term.grid().clone();` (bounded: alternate screen has no history).
  2. `term.swap_alt();` — while in alt mode this only swaps, it does not reset.
     The normal grid (with history) is now `term.grid()`; render steps 3-4.
  3. `term.swap_alt();` — switches back; this resets the alternate grid and
     overwrites its cursor.
  4. `*term.grid_mut() = alt;` — puts the alternate grid back exactly.
  Pin this with tests: after a restoration taken while in alt mode the alternate
  screen content, cursor and saved cursor are unchanged, and when the application
  later leaves the alternate screen the normal content, history, cursor, **saved
  cursor** (`swap_alt` writes `grid.saved_cursor`) and keyboard modes are as they
  were. Never do this while a synchronized update is
  pending.
- **Front filter / tokenizer** (pure, in `climon2-vt`): splits PTY output into
  complete tokens: printable text runs (never split inside a UTF-8 sequence), C0
  controls, ESC sequences, CSI sequences, and control strings. Mirror the state
  machine in `vte-0.15.0/src/lib.rs` exactly: OSC ends on BEL or ST (`ESC \`);
  DCS/SOS/PM/APC end on ST only (BEL is payload); an ESC not followed by `\` ends
  the string and starts a new escape; CAN/SUB abort; C0 controls inside CSI are
  executed. Differential-test token boundaries against vte on tricky inputs. Partial tokens
  are buffered across reads (bounded by the 4096 cap). For each complete token:
  drop it entirely if over the cap; otherwise feed it to the engine (in <= 64-byte
  steps) and append it to the live output unless it is a stripped query (1.9).
  Because snapshots are taken only at token boundaries, the engine and host are
  both in ground state when live forwarding begins for a new controller.
- **Eviction detection** (for the notice): only when `max_rows > 0`. Before each
  engine step, if `history_size() == max_rows`, remember the topmost history row;
  after the step, if history is still full and that row differs, set `evicted`.
  Clear `evicted` when `history_size()` drops to 0 (history cleared). Resize reflow
  that evicts rows also sets it. Coarse by design: it does not measure what was lost.
- **Serializer** (pure functions over a read-only grid view): build SGR from cell
  attributes (named colours -> 30-37/90-97/40-47/100-107/39/49, indexed -> `38;5;n`,
  RGB -> `38;2;r;g;b`, underline colour `58:…`), emit `ESC [ 0 m` at row starts,
  skip wide-char spacers, append zero-width characters, trim trailing default blank
  cells on hard-ended rows, print soft-wrapped rows full width. Titles use 1.8.1.
- Keep functions tiny; table-drive SGR and mode emission so CRAP stays <= 5.

### 3.3 Daemon structure

- **One writer**: the existing single-threaded `Session` loop owns the PTY, the
  `Vt`, all connections and a new pure **control state machine**. No other thread
  touches them.
- **Control state machine** (new pure module, e.g. `crates/climon2-daemon/src/control.rs`):
  inputs are events, outputs are effect lists; it owns lifecycle, attachments,
  controller, hand-back recency order, control period, readiness, per-client
  remembered size, applied session size and shutdown state.

  ```rust
  enum Input { Attach{conn, size}, TakeControl{conn}, Ready{conn, period},
               Resize{conn, size}, Detach{conn}, Lost{conn}, Kill{conn},
               InitialDeadline, CommandExited }
  enum Effect { Grant{conn, period}, Revoke{conn}, ResizeSession(size),
                AcceptDetach{conn}, Refuse{conn, reason}, EnableInput{conn},
                DiscardPendingInput, BeginShutdown, ... }
  ```

  Rules from sections 1.5-1.7 live here and nowhere else. Expose
  `snapshot() -> State` for hello. Unit-test it exhaustively (it is the easiest
  place to kill mutants).
- **Loop changes** (`session.rs`):
  - Read the PTY every tick regardless of clients (bounded per tick, e.g. one
    32 KiB read), feed `Vt`, write replies to the PTY, and enqueue `live` bytes to
    the controller only when its restoration has been enqueued.
  - On `Grant`: apply size (`ResizeSession` -> `pty.resize` + `vt.resize`), then
    when `!vt.sync_pending()` and `vt.at_boundary()`, enqueue `Granted{period}`,
    the restoration as `Output` frames (<= 65,536 bytes each) and
    `RestoreEnd{period}`. Call `vt.tick(now)` every loop.
  - On `Revoke`: enqueue `vt.release()` as `Output`, then `Revoked`; stop live output.
  - Input frames are written to the PTY only from the ready controller of the
    current period; `DiscardPendingInput` drops unwritten input on transfer.
  - Enforce the 16 MiB queue and restore delivery deadlines in `connection.rs`;
    violations become `Lost{conn}`.
  - Initial deadline: 5 s from readiness while `starting` -> `InitialDeadline`.
  - Shutdown needs restructuring, not just reuse. Today `daemon/src/lib.rs`
    `serve` returns from `session::serve` (dropping every connection) and only then
    calls `Resources::cleanup`; and `Pty::leave` discards the command's exit status
    and blocks the loop with `thread::sleep(HANGUP_GRACE)`. Change it so that:
    the hangup grace is a loop deadline (the loop keeps answering hello with
    `terminating` and refusing attaches); the actual exit status from hangup/kill
    is captured; the session performs `Resources::cleanup` itself **while
    established connections stay open** (sending on an accepted Unix stream after
    unlinking the socket path is valid); only then send `Exit{code}` to attachments
    and `Terminated{code}` to kill requesters (never `Terminated` if cleanup failed);
    flush all connections non-blockingly under one shared short deadline,
    prioritising kill requesters, then exit. Keep the existing normal-exit drain.
  - Hello replies include `capabilities` and `state`.
- `args.rs`: required `--scrollback-max-rows`; title default built from the argv
  after `--`.

### 3.4 Protocol additions (version-private, `climon2-core/src/protocol.rs`)

Keep existing codes and framing; extend the frame-type validation match and the
data-length rule.

| Code | Direction | Body | Meaning |
|---|---|---|---|
| `0x10` Attach | C->D | `{"cols","rows"}` | attach and take control (existing code, new semantics) |
| `0x11` Input | C->D | raw | application input (applied only if ready controller) |
| `0x12` Resize | C->D | `{"cols","rows"}` | remembered; applied only for controller |
| `0x13` TakeControl | C->D | `{}` | takeover request |
| `0x14` Ready | C->D | `{"period":u64}` | restoration written; enable input |
| `0x15` Detach | C->D | `{}` | explicit detach request |
| `0x16` Kill | C->D | `{}` | kill-mode request (instead of Attach) |
| `0x20` Attached | D->C | `{}` | attachment accepted (non-controlling until Granted) |
| `0x21` Output | D->C | raw | restoration or live bytes |
| `0x22` Exit | D->C | `{"code":u32}` | command finished |
| `0x23` Granted | D->C | `{"period":u64}` | this client controls; restoration follows |
| `0x24` RestoreEnd | D->C | `{"period":u64}` | restoration complete |
| `0x25` Revoked | D->C | `{}` | lost control; show placeholder |
| `0x26` Detached | D->C | `{}` | detach accepted |
| `0x27` Terminated | D->C | `{"code":u32}` | kill confirmed after cleanup |
| `0x7f` Error | both | `{"message"}` | refusal / protocol error |

Update `ConnectionState` so: after hello a connection may send `Attach` or `Kill`
(not both); `TakeControl`, `Ready`, `Detach`, `Input`, `Resize` require an
accepted attachment. The client sends `Ready{period}` only after writing every
byte up to `RestoreEnd{period}` to stdout; stale `Ready` (old period) is ignored.
Per-connection FIFO ordering plus `Ready` rejects stale socket input without
tagging every input frame. Keystrokes typed during restoration can still sit in
the local terminal's input queue, so the client must, at the moment it sends
`Ready`: discard pending terminal input (`tcflush(stdin, TCIFLUSH)` through a
`climon2-platform` seam), reset its input-filter state, then start forwarding.
Also reset input-filter state (pending prefix, partial paste marker) on every
`Granted` and `Revoked`.

### 3.5 Client structure

- Startup: parse envelope (or load config if absent) before raw mode; hello;
  validate identity/version/protocol **and** capabilities; send `Attach` or `Kill`.
- Pure **input filter** (`keys.rs`): state = {escape pending, in paste, partial
  marker buffer}; `feed(bytes, role) -> Vec<Action>` where role is
  Controlling-ready / Pending / NonControlling and actions are
  `Forward(bytes)`, `Detach`, `TakeControl`. Pending and NonControlling discard
  application bytes.
- Pure **client state** (role, current period) driven by server messages:
  `Granted` -> Pending; after stdout drained past `RestoreEnd` -> send `Ready`,
  role Controlling; `Revoked` -> draw placeholder, role NonControlling.
- Placeholder drawing: move to bottom row and emit `rows` line feeds (preserve
  visible content), `CSI H`, then the two lines. Redraw on SIGWINCH while
  non-controlling.
- Detach: send `Detach`, wait for `Detached` (bounded, e.g. 5 s), restore
  termios, print the stderr line, exit 0. Before restoring termios on any exit
  path where the daemon sent `release()` bytes, write them first; on unexpected
  loss write a conservative reset (`CSI 0 m`, `CSI ? 25 h`, mouse/paste/focus
  modes off) — never `CSI ? 1049 l` blindly, since the client does not know the
  screen state.
- Kill mode: no raw mode; send `Kill`, wait for `Terminated{code}` -> exit 0,
  `Error` -> 125, connection loss -> 125.
- Release bytes (`Vt::release`): end any open synchronized update
  (`CSI ? 2026 l`); if alternate screen active `CSI ? 1049 l`; reset margins
  (`CSI r`), origin/insert/line-feed modes off, autowrap on; turn off mouse, focus,
  bracketed paste, application cursor/keypad; show cursor; `CSI 0 m`; default
  cursor style.

### 3.6 CLI structure

- `cli.rs`: add `Attach { selector }`, `Kill { selector }` (parse selector shape
  only; usage error 2 on malformed or missing).
- New `attach.rs` / `kill.rs` sharing a `dispatch.rs`: discover (reuse
  `list::discover`, extend it to return parsed hello extensions), select, check
  capabilities and client path, then:
  - attach: TTY check, self-attach check, `config` client snapshot, `exec` the
    client with the envelope.
  - kill: spawn the client with the kill envelope (stdin/stdout/stderr suitable for
    a non-TTY), wait until the 10-second deadline, kill and reap only that child on
    expiry.
- `exec.rs`: load config (client + scrollback) first; pass `--scrollback-max-rows`;
  strip `CLIMON2_CLIENT_STARTUP` from the daemon/application environment
  (`SessionEnvironment` projection); set the attach envelope when exec'ing the client.
- `list.rs`: VERSION/STATE/CLIENTS columns and JSON `state`.

## 4. Increments (each one: red test, green, refactor, gates, commit, push)

Run `cargo xtask test` at least once per increment and the quality gate on changed
shipped source (section 7) before committing.

1. **Baseline.** Confirm architecture (`uname -m`); select bash 5.x via
   `CLIMON2_TEST_BASH` (e.g. `/usr/local/bin/bash` or Homebrew bash). Run
   `cargo xtask test`; record pre-existing failures/environment gaps in the action
   log. No commit unless something is fixed.
2. **Bash 5-only interactive qualification.** `climon2-test-term/src/shell.rs`:
   accept only GNU bash 5.x; probe `CLIMON2_TEST_BASH`, `/opt/homebrew/bin/bash`,
   `/usr/local/bin/bash` (not `/bin/bash`). Update `climon2-fixtures/tests/bash.rs`,
   packaged tests that iterate `bash_profiles()`, CI matrices (drop interactive
   `3.2` from both the `macos` and `archive` jobs), `README.md` and
   `crates/climon2-e2e/README.md`. Keep `install.sh` and any installer/
   non-interactive `/bin/bash` uses unchanged and tested.
3. **Core contracts (pure).** `config.rs`: a lexical normaliser that respects
   strings and escapes and (a) removes `//` and `/* */` comments, (b) removes a
   comma whose next non-whitespace/non-comment character is `}` or `]`; then
   `serde_json` into **operation-specific** structs: `ExecConfig { client,
   scrollback }` and `AttachConfig { client }` (the attach struct does not name
   `scrollback`, so serde ignores it as unknown and an invalid unused value cannot
   fail attach). serde derive rejects duplicate known fields; use a three-state
   field type to distinguish missing / `null` / value. Test with the exact sample
   in 1.3 (comments and trailing commas) and with an invalid `scrollback` during
   attach. Then key vocabulary, selector parse/match, startup envelope, hello extensions
   (lenient optional parsing: parse mandatory `HelloReply` as today and the optional
   fields separately from a `serde_json::Value`), state consistency rules, title
   encoding. Exhaustive unit tests, including the exact 4096-byte envelope boundary.
4. **Protocol messages.** New codes and `ConnectionState` rules with framing tests.
5. **`climon2-vt`.** Crate + gate registration first. Then tokenizer (split-across-
   reads cases, cap, CAN/SUB, UTF-8 boundaries, stripped query list), engine feed
   and replies, title tracking, eviction flag, serializer, alternate-screen
   restoration and the swap pin test. Oracle for serializer tests: feed the source
   bytes into `Vt`, feed `restoration()` into a **fresh** Alacritty `Term` (a test
   dependency of `climon2-vt` is fine) and compare visible text, rendition, cursor,
   history rows and wrap flags.
6. **Daemon state machine** (pure) with exhaustive tests of 1.5-1.7.
7. **Daemon integration.** Continuous PTY reading, VT feeding, replies, grants and
   restoration delivery, revocation, readiness gating, detach/kill/initial
   deadline/shutdown, hello extensions, queue and restore deadlines, required
   `--scrollback-max-rows`. Test through the real daemon binary with raw-socket
   fake clients (extend `crates/climon2-daemon/tests/lifecycle.rs` style) so control
   periods, stale input and ordering are asserted byte-exactly.
8. **Client.** Envelope/config startup, input filter, roles, placeholder, detach,
   kill mode, exit statuses (`crates/climon2-client/tests/bridge.rs` style).
9. **CLI.** `attach`, `kill`, selector errors, capability/client-path checks (use a
   fake hello server or a controlled-version install to simulate an older session
   without capabilities), self-attach, config scope per operation, envelope
   overwrite and exclusion from daemon/app env, deterministic config-edit race
   (edit the file after validation but before the client starts: the client must use
   the old snapshot; the daemon must keep its retention), list columns/JSON.
10. **Packaged product tests** (section 6).
11. **Docs, native checks, full gates, push** (sections 8-10).

## 5. Test harness changes

- `climon2-test-term`: add an option to **disable automatic `PtyWrite` replies**
  (e.g. `Terminal::spawn_with(…, Replies::Off)`) so tests prove the daemon answers
  queries; add access to the emulated window title (track `Event::Title`/
  `ResetTitle`) and to scrollback text (history rows) for restoration assertions.
- Fixtures: extend `climon2-fixture`/`climon2-tui-fixture` only as needed, e.g.
  a mode that sends DA1/DSR and prints the reply it received (with a timeout), a mode
  that waits for an acknowledgement file/byte before printing (to produce output
  while detached deterministically), a mode that writes a large volume (tens of MiB)
  and reports completion, and a mode that sets a title. Use acknowledgements and
  observable completion for ordering; sleeps are never proof of readiness.

## 6. Packaged product acceptance (`crates/climon2-e2e/tests/packaged.rs`)

Installer -> launcher -> CLI -> daemon -> client, real controlled bash 5 outer
terminals, isolated homes. Each test asserts actual exit statuses and ends with
`assert_clean()` (no owned processes, sockets or metadata left).

1. Detach and reattach: shell output and history before detach; Ctrl+] d exits 0
   with the stderr line and `climon2 attach <id>`; output produced while detached;
   `list --json` shows `detached`/0; attach by 8-hex prefix restores history, the
   detached output and the prompt; typing works; `exit 7` returns 7 to the
   attached client.
2. Alternate screen: TUI fixture (or `vim`/`less`) in alt screen; detach; reattach
   shows the alt content; quitting the TUI shows the restored normal content.
3. Multi-client: A execs, B attaches (A shows placeholder with `Press Ctrl+T to
   take control`), A takes control with Ctrl+T, B's input is discarded while
   non-controlling, B detaches, A continues; controller size drives the session
   (fixture reports PTY size); command exit returns the status to all attached
   clients.
4. Unexpected final loss: kill the only client process -> session terminates and
   cleans up (adapt the existing `killed_installed_client_uses_daemon_leave_policy…`).
5. Kill: detached session killed silently with 0; attached session killed while a
   client is attached (client receives the signal-derived status); unknown and
   ambiguous selectors 125; malformed 2.
6. Self-attach: inside a session, `climon2 attach $CLIMON2_SESSION_ID` -> 125 and
   the session/controller are undisturbed.
7. Configuration: invalid config -> `exec` 125 before any session exists;
   custom escape prefix works; `take_control_key: null` hides the instruction;
   `max_rows` small -> reattach shows the truncation notice; `max_rows: 0` restores
   the screen without history.
8. Query authority: with harness replies disabled, a fixture's DA1/DSR receives
   exactly one reply, both attached and while detached.
9. Title: default `climon2 exec <cmd> <args>` visible in the harness title after
   attach; an application title with a control byte is restored percent-encoded.
10. Detached throughput: a large write completes while detached; the session stays
    live.
11. Existing tests keep passing (vim, less, TUI queries/mouse/paste/resize, raw
    socket abuse, version routing, log privacy, stale cleanup).
12. Previous-release compatibility: the existing
    `actual_previous_release_remains_live_across_current_archive_install` gate stays.
    No release exists yet, so it is a no-op locally; never fabricate a previous
    archive. Synthetic controlled versions cover unsupported-operation paths.

## 7. Quality gates and commands

- `cargo xtask test` — fmt, strict clippy (`-D warnings`), ShellCheck, all tests.
- `cargo xtask setup-quality` once, then `CLIMON2_MUTATION_BASE=<base> cargo xtask mutate`
  for changed shipped source during increments, and `cargo xtask check` before
  pushing. `cargo xtask check --mutate-all` before declaring the work complete
  (long-running; run it in the background).
- `cargo xtask fixtures-check` (append-only goldens), `cargo xtask release-check`.
- `cargo xtask dist` requires an arm64 macOS host. If the local host is Intel, rely
  on CI (`dist` and `archive` jobs run on both architectures) and report it.
- New shipped crate: add `crates/climon2-vt/src` to `SHIPPED`; confirm crapper and
  mutator report its functions. Never weaken thresholds, add exclusions or mark
  mutants as acceptable.

## 8. Documentation updates

- `README.md`: usage (attach, kill, keys, detach/takeover, placeholder),
  replace "Leaving the client ends the session; detach, reattach and screen
  restoration are not supported" with the new leave policy, configuration file,
  list columns, limitations (unmarked paste, binding conflicts with applications,
  native duplicates on reattach, unrestored state listed in 1.8, self-attach only,
  other cycles are the user's responsibility, legacy-only keyboard input), shell
  matrix (bash 5.x interactive only; `/bin/bash` installer compatibility).
- `GLOSSARY.md`: attachment, controller, control period, detach, take control,
  hand-back, restoration, retained history, placeholder, leave policy,
  capabilities, startup envelope.
- `docs/adr/`: add an ADR recording the daemon-owned virtual terminal (Alacritty
  in `climon2-vt`, eager restoration, live filtering/query authority) and update
  the crate-layout ADR (0004) for `climon2-vt`.
- Crate READMEs (`crates/climon2-client/README.md`, `crates/climon2-e2e/README.md`).
- Mark `docs/agents/session-continuity-handoff.md` as superseded by this plan.

## 9. Native sanity checks (character presence only)

- Terminal.app: open a fresh window via `osascript` running a script with isolated
  `HOME`/`CLIMON2_HOME`; run `climon2 exec bash`, print markers, detach, reattach,
  and read the window's text through AppleScript (`contents`/`history` of the
  run's own tab) to confirm the markers and restored history are present; close
  only that window.
- VS Code: `code --new-window --user-data-dir <tmp> --extensions-dir <tmp>` with
  an integrated terminal running the same script. Only proceed if text can be
  observed without screenshots or the clipboard; otherwise report the check as
  blocked.
- If macOS permission prompts (Automation/Accessibility) block automation, report
  the check as blocked; do not change system settings.
- No pixel, palette or glyph-shape assertions.

## 10. Finish and report

- All increments committed and pushed to `main`; CI green on both
  `macos-15` (arm64) and `macos-15-intel` jobs (watch with `gh run watch`). Never
  substitute one architecture for the other; report an unavailable runner as an
  open gate.
- Final report: exact behaviour completed, tests added, gates run and their
  results, outstanding environment/architecture gates, native check outcomes, and
  the action log (every action and why).

## 11. Pitfalls

- Self-attach detection belongs in the CLI preflight. The exec'd client inherits
  the CLI's environment (not the projected session environment); keep the
  startup envelope out of the daemon/application environment.
- Today the daemon stops reading the PTY when the controller is slow; continuity
  requires continuous reading with the 16 MiB per-attachment queue instead.
- The existing frame-type validator rejects unknown codes before reading the body;
  add new codes there or frames will be rejected.
- `HelloReply` uses serde defaults (unknown fields ignored); keep mandatory field
  parsing strict and optional extensions lenient so bad optional data never makes a
  live session look stale (stale cleanup deletes metadata).
- Snapshot only at token boundaries and outside synchronized updates, otherwise the
  host starts live output mid-sequence.
- `Term::swap_alt` resets the alternate grid when entering alt mode; always restore
  the cloned alternate grid afterwards.
- Logs must not contain terminal content, titles, arguments or environment values.
- Keep each function tiny: crapper counts `?` as a branch, and every delegation or
  constant must be killed by a test.
