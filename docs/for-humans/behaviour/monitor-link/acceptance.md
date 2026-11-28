# Monitor link acceptance criteria

**Status: developer-confirmed specification for
[Define the automated acceptance criteria for monitor links](https://github.com/jackgeek/climon2/issues/481),
not executed acceptance evidence.** No release contains monitor links.
They are being built ticket by ticket on a feature branch of
[#488](https://github.com/jackgeek/climon2/issues/488), whose pull requests
name the scenarios they prove.

This document says what automated evidence shows that monitor links meet the
decisions recorded on
[Chart the monitor link specification](https://github.com/jackgeek/climon2/issues/189).
It follows the [monitor's acceptance criteria](../monitor/acceptance.md) and
reuses their evidence policy.

## Evidence policy

The monitor's [evidence policy](../monitor/acceptance.md#evidence-policy)
applies unchanged: stable scenario IDs, cumulative layers, tests that fail
when the promised behaviour is absent, independently specified expectations
for the [terminal oracle](../../../../GLOSSARY.md#testing), and test hooks
that never ship. Its Contract, Monitor, Fault, Measure and Docs layers keep
their meaning. Monitor links add three layers and two test tools.

| Layer | Required meaning |
|---|---|
| Link | The link code, in both roles, driven over in-memory byte streams. The test supplies the time and the randomness. No socket, file or process is involved. |
| Relay | The actual packaged `climon2-relay`, started with `c2 relay` and driven through a pseudo-terminal, with real session daemons behind it. |
| Pair | A packaged hub monitor and a packaged relay on one host, each in its own climon2 home, joined over loopback TCP, with real session daemons behind the relay. |

The **link code** is the part of climon2 that performs pairing, the
handshake, framing and relayed records over a plain byte stream. It lives in
its own crate, `climon2-link`.

Pair is the only end-to-end layer. No scenario needs a second machine or a
second operating system. Relay and Pair scenarios run on x86-64 GNU/Linux and
both macOS architectures, as the monitor's scenarios do.
[What Windows runs](#what-windows-runs) covers Windows.

Two test tools stand for every attacker in the threat model. Both belong to
the Fault layer and neither is shipped.

- The [on-path proxy](../../../../GLOSSARY.md#testing) sits between the two
  ends on loopback. It can drop, delay, reorder, replay, edit and inject
  bytes. No operating-system network tooling is used.
- The [link peer stand-in](../../../../GLOSSARY.md#testing) speaks the link's
  bytes itself, as a hub monitor or as a relay, with one deliberate defect.

### Durations

The specification fixes many durations. Each is proven exactly at the Link or
Contract layer, with the test supplying the time. Relay and Pair scenarios
wait in real time only for durations of 15 seconds or less. For longer ones
they use a clock hook built only into test builds, as the monitor's
deterministic daemon clock is. No release artifact carries the hook.

### Blanket checks

After every Relay and Pair scenario the harness makes two checks, whatever the
scenario was about:

- **Secrets.** It scans every log, the links status file, every file under
  `links/` that is not a key file, and the hub monitor's captured terminal
  output for the PIN and for the bytes of every private key and session key
  the run used. The PIN may appear only in the relay's invitation box and at
  the hub monitor's PIN prompt.
- **Permissions.** `links/` is owner-only (`0700`) and every file in it is
  owner-only (`0600`).

### Sources

Each scenario names the decisions it covers by a key and an item number, so
`P10-12` means items 10 to 12 of the pairing resolution. Every numbered item
of every resolution below has at least one scenario, except those listed
under [Decisions with no scenario](#decisions-with-no-scenario).

| Key | Resolution |
|---|---|
| T | [Threat model and security goals](https://github.com/jackgeek/climon2/issues/192#issuecomment-7163864321) |
| P | [Link invitation and PIN pairing ceremony](https://github.com/jackgeek/climon2/issues/193#issuecomment-7167131871) |
| X | [Relay executable's shape and lifetime](https://github.com/jackgeek/climon2/issues/194#issuecomment-7167833345) |
| A | [Session advertisement and how a relay carries daemon traffic](https://github.com/jackgeek/climon2/issues/195#issuecomment-7169769180). Its decisions are not numbered, so scenarios cite it whole. |
| V | [Version compatibility across a monitor link](https://github.com/jackgeek/climon2/issues/196#issuecomment-7170358970) |
| U | [Hub monitor authority and relay-side consent, visibility and cut-off](https://github.com/jackgeek/climon2/issues/197#issuecomment-7175111392) |
| L | [What a lost monitor link means for sessions the hub monitor controls](https://github.com/jackgeek/climon2/issues/198#issuecomment-7176810208) |
| S | [Remote sessions and link state in the selector](https://github.com/jackgeek/climon2/issues/199#issuecomment-7225800159) |
| ST | [How remembered pairings are stored and forgotten](https://github.com/jackgeek/climon2/issues/407#issuecomment-7227004326) |
| RT | [The relay's terminal interface](https://github.com/jackgeek/climon2/issues/408#issuecomment-7228080820) |
| LM | [Limits and accounting for remote sessions and the relay](https://github.com/jackgeek/climon2/issues/412#issuecomment-7227371018) |
| C | [A session's clipboard copies in the monitor](https://github.com/jackgeek/climon2/issues/414#issuecomment-7227768450) |
| D | [The hub monitor's links dialog](https://github.com/jackgeek/climon2/issues/474#issuecomment-7228329441) |
| K | [Keepalive, dead-link detection and redial backoff](https://github.com/jackgeek/climon2/issues/477#issuecomment-7228501211) |
| CM | [Command and configuration surface for relays and links](https://github.com/jackgeek/climon2/issues/478#issuecomment-7228779376) |
| R | [What the relay and hub monitor record about links](https://github.com/jackgeek/climon2/issues/479#issuecomment-7228936965) |
| W | [The link protocol's records for link revision 1](https://github.com/jackgeek/climon2/issues/480#issuecomment-7229751020) |

The messages and records these scenarios name are specified byte for byte in
the [monitor link protocol](../../protocol/monitor-link/README.md) for link revision 1. Its
[cryptography](../../protocol/monitor-link/README.md#cryptography) section names the two
constructions: CPace, the key exchange that pairing runs on the PIN, and
Noise `IK`, the handshake a remembered pairing uses. The **opening exchange**
is the [request](../../protocol/monitor-link/opening-request.md) and
[reply](../../protocol/monitor-link/opening-reply.md) that start every connection, unencrypted, and
pick the [link revision](../../../../GLOSSARY.md#monitor-links). A
[unit](../../protocol/monitor-link/README.md#units) is one encrypted piece of a link, and a **record**
is one message carried in units. The protocol's
[limits](../../protocol/monitor-link/README.md#limits) and [timings](../../protocol/monitor-link/README.md#timings) sections
hold the numbers the scenarios quote.

## Pairing

| ID | Setup and action | Required observable result | Decisions | Layers |
|---|---|---|---|---|
| PAIR-01 | Mint an invitation on the relay. Pair from the links dialog and, separately, with `c2 monitor pair`, typing the PIN with and without hyphens and spaces. | Pairing takes four messages on its own connection, which carries no session traffic. Each side shows the other's name and key fingerprint. The hub monitor reports "paired" only after its first pinned-key handshake succeeds. The invitation is consumed. The hub monitor used a fresh keypair; the relay kept its one identity key, added a remembered hub monitor and evicted none. | P1, P4-5, P15-17, P19-23, W6, D8, CM15 | Link, Pair |
| PAIR-02 | Mint, then mint again. Let an invitation reach 5 minutes. Cancel one with Escape. Pair once, then try the same PIN again. | A relay has one live invitation: a new one revokes the old. Expiry at 5 minutes is final and nothing is minted automatically. The pair box stays in the foreground and reports paired, expired, used up or cancelled. An invitation pairs one hub monitor. | P6-9, X6, RT10 | Link, Relay |
| PAIR-03 | Fail pairing three times with a wrong PIN. Separately, after the relay's share, hang up, time out and send a malformed message. Before the relay's share, connect and close, send garbage, and open with no shared link revision. Start a second pairing run while one is in flight. | Every failure after the relay's share counts; none before it does. The third failure destroys the invitation. A concurrent run is refused at once and does not count. A run that passes 30 seconds, or sends an oversized message, fails. The relay reports each failed attempt with the attempts left and the source address, marked unauthenticated. | P10-12, P24, V18 | Link, Relay, Fault |
| PAIR-04 | From the dialog and from the command, pair with an unreachable address, with a relay that has no invitation open, and with a wrong PIN. | The hub monitor names which of the three failures happened. The dialog lets the user try again from the same form. The command reports them by message on stderr, all with status 125. | P25, D7, CM24 | Monitor, Pair, Fault |
| PAIR-05 | A stand-in dialler without the PIN runs pairing. A stand-in relay without the PIN accepts a hub monitor's pairing. | The dialler receives the relay's share and never a confirmation, because the hub monitor confirms first. Neither stand-in receives anything that tests more than one guess per run, and the PIN's digits never cross the wire. | P2, T16 | Link, Fault |
| PAIR-06 | The on-path proxy edits either opening message, swaps the two roles' messages, or splices the shares of two runs. | Pairing fails, counts as an attempt and pins nothing on either side. | P3, T17 | Link, Fault |
| PAIR-07 | Stop the hub monitor after the relay has verified its confirmation and before its first pinned-key handshake. | The relay consumed the invitation and keeps an entry its user can forget. The hub monitor never reported "paired". | P20 | Link, Pair, Fault |
| PAIR-08 | Mint while a hub monitor is linked, and while on hold. Let the invitation fail, expire and be cancelled. Then pair a second hub monitor successfully. | Minting and an unsuccessful invitation change nothing: the link stays up, or the hold stays. A successful pairing lifts a hold and closes the current link with "replaced by a newly paired hub monitor". The relay then admits only the new hub monitor until it links or 30 seconds pass. The displaced one stays remembered and afterwards gets the ordinary busy refusal. | U10-12 | Link, Pair |
| PAIR-09 | Pair with a relay already remembered. Pair again with a relay that forgot this hub monitor. Pair when the relay's suggested name is taken. Run `pair` with a PIN that is not 8 digits, a `--name` already in use, and a full store of 64 relays. Fill the relay's 64 remembered hub monitors and pair a 65th. | A known relay's record is replaced and keeps the user's name and address. Pair again asks only for the PIN. The dialog refuses a taken name and asks for another; the command takes the first free suffix and says so. The three bad inputs fail with 125 before anything is dialled, so no attempt is spent. A full relay refuses the new pairing with an explicit message. | D9-10, CM11-14, LM4 | Contract, Monitor, Pair |
| PAIR-10 | Run `c2 monitor pair` with stdin a terminal and with stdin a pipe. Try to supply a PIN by flag, environment variable and setting. | With a terminal it prompts `PIN: ` on stderr and echoes the typing. Otherwise it reads one line from stdin. No flag, variable or setting carries a PIN. The address the user gives is used as typed and remembered, never checked against the invitation's hint. | P13-14, D6, CM10 | Contract, Pair |

## Link security

| ID | Setup and action | Required observable result | Decisions | Layers |
|---|---|---|---|---|
| SEC-01 | Link twice with the same remembered pairing and compare the two connections. | Both ends authenticate each other with Noise `IK` over X25519, ChaCha20-Poly1305 and SHA-256. Each connection uses fresh ephemeral keys, so the two share no session key and no ciphertext. | T2-3, W1-2 | Link, Pair |
| SEC-02 | The on-path proxy flips a bit in, truncates, drops, reorders, duplicates and injects units on an established link. | No altered, repeated or out-of-order unit is delivered to either end as a record. The link ends and the hub monitor redials. | T2 | Link, Pair, Fault |
| SEC-03 | Replay a recorded first handshake message to the relay, alone and while that hub monitor is linked. Replay a whole recorded connection. | The relay treats nothing as linked, replaces no existing link and changes no state, because the hub monitor's first record never arrives. A replayed connection delivers nothing. | W3 | Link, Fault |
| SEC-04 | Send the relay a malformed, partial or late opening message. Complete the handshake as a stand-in with a key the relay does not remember. Dial a stand-in relay that holds the wrong key. | A bad or late opening is closed without a byte in reply, after at most 10 seconds. The unknown key gets a Close with "not paired" and nothing else. The wrong relay receives nothing that identifies the hub monitor, and no record. Before authentication the wire carries only the fixed marker, the purpose and link revision numbers. | T15, V6-7, V9, W4-5 | Link, Fault |
| SEC-05 | While a hub monitor is linked, open unauthenticated connections up to and past the limit of 8, hold each open, and forge an opening refusal on a new dial. | The established link is never cut or delayed. The ninth connection is refused at once, and each held one is closed at 30 seconds. A forged refusal costs the dialler one dial and nothing else. | T5, V19, LM8 | Link, Relay, Fault |
| SEC-06 | Move the relay to another address and port and edit the address on the hub monitor. Put a different relay at the remembered address. Link successfully through a second address. | The moved relay links with no PIN. The different relay fails the handshake and the record is unchanged. A link that succeeds on a different address replaces the stored one. An address is never evidence of identity. | T13-14, ST12 | Link, Pair, Fault |
| SEC-07 | As a second local user on either machine, read `links/`, the logs and the configuration, and connect to the relay's listener. | The files are unreadable. The listener gives that user no more than SEC-04 and SEC-05 give any network attacker. | T6 | Pair, Fault |
| SEC-08 | Run the link code against published and frozen known answers: the published CPace vectors for ristretto255 with SHA-512, a published Noise `IK_25519_ChaChaPoly_SHA256` vector, and climon2's own frozen vectors for the pairing key schedule and for binding the opening exchange into both key exchanges. | The link code produces the expected bytes exactly. Replacing or upgrading a cryptographic crate cannot change them unnoticed. | T12, W1 | Link |
| SEC-09 | Pair with a stolen invitation before its owner does, then let the owner try. Forget a hub monitor on the relay, then dial with its key. | The first pairing consumes the invitation and the relay shows who paired, by name and fingerprint. The owner's attempt fails. A forgotten key gets only "not paired". | T8, T10-11 | Link, Pair |
| SEC-10 | Run every Relay and Pair scenario. | The [blanket checks](#blanket-checks) pass. | T18, P26, R16-17, ST1, ST4 | Relay, Pair |

## Link revisions

| ID | Setup and action | Required observable result | Decisions | Layers |
|---|---|---|---|---|
| REV-01 | Dial with a stand-in offering link revisions 1 and a higher unknown one, in each order. Link the same pairing repeatedly. | The opening request is 26 bytes and the reply 20, with the specified layout. The ends use the highest revision they share. Every connection negotiates from scratch and no record of a past revision is kept. | V1, V6, V10, W4 | Link, Fault |
| REV-02 | Dial with a stand-in that shares no link revision, as hub monitor and as relay, for a link and for a pairing. | The link is refused as incompatible. Each end shows both revision lists and names the machine to upgrade: `Upgrade` on the hub monitor's rows, Incompatible on the relay's status bar. The pairing is kept, and the hub monitor redials every 60 seconds. During pairing, the invitation and its attempts are untouched. | V15-18, RT9 | Link, Monitor, Relay, Fault |
| REV-03 | The on-path proxy edits the revision lists in either opening message, on a link and on a pairing connection. | The key exchange that follows fails. Neither end falls back to another revision. | V8, T17 | Link, Fault |
| REV-04 | Send a record with an unknown tag. Send a Hello with unknown capability tokens. | The unknown tag is a protocol error that ends the link and is never skipped. Unknown tokens are ignored. Tokens travel only in Hello, inside the encrypted link, and link revision 1 sends none. | V3-4, W9 | Link, Fault |
| REV-05 | Link with stand-ins that report older, newer and malformed release versions. | Release versions cross only in Hello, inside the encrypted link, and are shown. No behaviour differs between the runs. | V14 | Link, Pair, Fault |
| REV-06 | On the relay's machine, run sessions from the release under test and from the [second-release fixture](../../../../GLOSSARY.md#testing), then remove one session's version folder. Open each from the hub monitor, which has neither release's libraries for them. | Opening a stream states the client interface revision and passes the library's capability list unchanged. The relay loads each session's own release's client library once. A session with a missing version folder is still advertised; opening it is a failed start that names the cause, as for a local session. The hub monitor needs none of the relay's releases. | V11-13 | Relay, Pair |
| REV-07 | Replay every [protocol transcript](#protocol-transcripts) of every supported link revision. | Each end reproduces the recorded bytes exactly, in every test run of every release. | V2, V5 | Link |
| REV-08 | With a hub monitor linked, install the second-release fixture as the current version. Repeat with an invitation live, and with the relay on hold. | The relay closes the link with "restarting for an upgrade", shows the version it restarts into, and runs the new version in the same terminal with its original arguments. With an invitation live it waits, at most 5 minutes. The hub monitor reconnects without a PIN. A hold survives. If the new version fails to start, its error shows and nothing falls back. | X15-19, RT14 | Pair |

## Advertisement

| ID | Setup and action | Required observable result | Decisions | Layers |
|---|---|---|---|---|
| ADV-01 | Start a relay with live sessions and no link, then link, then start, change and end sessions. | Unlinked, the relay runs no discovery pass and makes no probe or connection to any session daemon. Discovery starts when the handshake completes. The hub monitor receives every session, then Snapshot complete, then one record per appearance, change and withdrawal. | X12-13, W13, A | Relay, Pair |
| ADV-02 | Advertise sessions and inspect the records. | Each carries the session id, command, arguments, working directory, depth, creation time and release version. The socket path, daemon process id, daemon start time and Linux daemon identity never cross the link. The working directory is display text only. | A | Link, Pair |
| ADV-03 | Start the relay inside a session at depth 1 and at depth 2, in a home with other sessions. | At depth 2 the relay refuses to start. At depth 1 it advertises every live session in its home except its own. There is no per-session filter. | X14, U1, U3 | Relay, Pair |
| ADV-04 | Make a session unreachable without proving it stale, then prove it stale. Do both for a row the hub monitor never connected and one it did. Drop the link and reconnect. | A session is withdrawn only on proof. A row never connected is dropped; one that was connected stays Disconnected with its history. After a reconnect the hub monitor discards what it held and takes the fresh snapshot. | A | Pair, Fault |
| ADV-05 | Give a local session and sessions on two relays the same session id. Reconnect and change a relay's address. | The three stay distinct rows and are never merged. A remote session keeps its identity, the relay's key plus the session id, across reconnects and address changes. | A | Link, Pair, Fault |
| ADV-06 | Run more than 128 live sessions in the relay's home. Advertise a 129th session from a stand-in relay. Advertise a session larger than the hub monitor's stated ceiling. Change one session many times while the hub monitor reads slowly. | An honest relay advertises the first 128 in its listing order and says on its screen how many are not served. A 129th is a link-level violation. An oversized session is not advertised. The relay keeps at most one pending change per session. Advertised listing data counts in the hub monitor's memory total. | LM3, W17 | Link, Relay, Pair, Fault |

## Session streams and parity

| ID | Setup and action | Required observable result | Decisions | Layers |
|---|---|---|---|---|
| STR-01 | On a remote session, take control, type, resize, kill and delete from the hub monitor. Look for any way to start a session on the relay's machine or reach its configuration or pairings. | Each action has the result it has on a local session. Delete is a Kill followed by a dismissal the hub monitor keeps for its own run; the relay deletes no metadata. No record starts a session or reaches the relay's settings. | U2, A | Monitor, Pair |
| STR-02 | Open, close and reopen streams. Drop the link and reconnect. Refuse a stream and watch the relay. | The relay creates a client instance only when the hub monitor opens a stream, tears it down on request and never retries by itself. There is one stream per client instance. Stream numbers rise from 1 and are never reused. A reconnect means new instances and new streams. Stream ended is always a stream's last record and carries the run report. | W14, A | Link, Pair |
| STR-03 | Send a large event on one stream while control records and another stream's events are queued. Replay every [content fixture](#content-fixtures). | Every unit is at most 16,384 bytes and carries one fragment of one record. Control records overtake the large event. Requests and events arrive complete and in order. Each client interface record is encoded by the one mechanical rule. | W7-8, A | Link |
| STR-04 | Open a stream to a session that was just withdrawn, and to one whose client library fails to load. | The stream reports a failed start, as a local client instance does. | A | Pair, Fault |
| STR-05 | Change a remote session's status and raise notifications before control, in the background, and during a link gap. | Status and notifications arrive through the session's own stream, as for a local session. Advertisement carries no status. After a reconnect the new client's initial metadata carries the current status. Notifications raised during the gap are not replayed. | A | Monitor, Pair |
| STR-06 | While the hub monitor controls a remote session, take it with a local monitor and with `c2 attach`. Repeat while the on-path proxy has silently stopped carrying bytes and the link is not yet declared lost. | The local client takes control every time: the most recent takeover wins. The hub monitor shows the placeholder. A local client that loses control to the hub monitor sees the ordinary viewed-elsewhere screen with no mention of a remote controller. | U14, and the standing requirement in L | Pair, Fault |
| STR-07 | Copy to the clipboard inside a remote session, with a copy at the size limit and one above it. Copy during a link gap. | The copy crosses as an ordinary session-stream record. The hub monitor applies its own 65,536-byte bound and offers the copy only through its consent key. A copy made during a gap is lost. | C11 | Monitor, Pair, Fault |

## Limits and credit

| ID | Setup and action | Required observable result | Decisions | Layers |
|---|---|---|---|---|
| LIM-01 | Link ends with different limits. Fill the hub monitor's limits with remote sessions, local sessions and a mix. | Neither end sends a limit to the other. The one stated number is the largest record each accepts, in Hello. A remote session counts against `monitor.limits` exactly as a local one does, with no separate budget, and reserves nothing for a client library. Each link reserves one descriptor and its buffers. | LM1-2, W9 | Contract, Monitor, Pair |
| LIM-02 | Start the relay with valid, invalid and absent `relay.limits`, and with fields that exist only for the monitor. Change the file while it runs. | The relay applies the same field names, defaults and validation as `monitor.limits` where a field applies, and reports errors explicitly. Its memory total defaults to 268435456 bytes. It reads the section once at start. `relay.limits` and `relay.listen` are the only new settings. | LM5-6, CM25 | Contract, Relay |
| LIM-03 | Exhaust the relay's admission, then open another stream. | That stream is refused with "relay out of resources". The hub monitor shows `Awaiting resources` and retries at its admission rate. The link and the other streams carry on. | LM7, W12 | Pair, Fault |
| LIM-04 | Stop the hub monitor draining one session while others run. Send events past the credit from a stand-in relay. | Credit counts the bytes of Event records and nothing else. A stream's credit is the free space in the hub monitor's event queue for that session. The relay buffers no more than the credit it holds and stops draining that client. Advertisement and control never wait behind session output. | LM10, W15, A | Link, Pair, Fault |
| LIM-05 | Leave one busy stream with no credit for three quarters of the relay's delivery stall deadline. | The relay sends a deliberate Leave on that client and ends the stream as fallen behind. The session keeps running. The hub monitor's ordinary passive retry reopens it. Other streams and the link carry on. | L4, LM11 | Link, Pair, Fault |
| LIM-06 | From a stand-in, break a per-stream bound. Separately send an oversized unit, a record above the receiver's stated ceiling, an advertisement flood, and more than 1,048,576 bytes or 1,024 records of unprocessed control traffic. | The per-stream breach ends only that stream, as a stream violation. Each of the others closes the link with an authenticated "protocol violation", and the hub monitor redials every 60 seconds. | LM9, W17, K8 | Link, Fault |
| LIM-07 | Flood a relay from many busy sessions with the hub monitor reading slowly. Run the monitor's [measurement job](../../../../GLOSSARY.md#testing) with remote sessions in place of some local ones. | The relay stays within `relay.limits`. The hub monitor's measured totals count remote sessions as local ones. No latency or throughput target is set. | LM6, LM2 | Measure |

## Link loss and recovery

| ID | Setup and action | Required observable result | Decisions | Layers |
|---|---|---|---|---|
| LOSS-01 | Leave a link idle. Have the on-path proxy stop carrying bytes in one direction. Send one large record slowly. Feed unauthenticated bytes to a silent link. | Each end sends a keepalive, an empty unit on stream 0, after 5 seconds of sending nothing. An end that receives no authenticated unit for 15 seconds declares the link lost. Each authenticated unit resets the timer, so the slow record keeps the link. Unauthenticated bytes do not. Neither number is a setting. | K1-4, W16 | Link, Pair, Fault |
| LOSS-02 | End a link in each way: loss, cut, hub monitor disconnect, replacement, clean relay stop and upgrade restart. Each time the hub monitor controls one session with a local attachment, one with a local monitor watching, and one with nobody else. | The relay sends a deliberate Leave on every client instance at once and waits for each daemon to accept it. No session ends. Control passes to the local attachment, else the local monitor's passive connection is promoted, else the session runs detached. The relay holds no instance through the gap. | L1-3 | Pair, Fault |
| LOSS-03 | End a link while one session daemon does not accept the Leave. | The relay gives up on that daemon after `deadlines.leave_ms` from `relay.limits`. Its other Leaves are not delayed. | L5 | Relay, Fault |
| LOSS-04 | Kill the relay outright while the hub monitor controls an ordinary session with no other participant, a headless session, and a session with a local attachment. | The ordinary session ends, as it does when a monitor is killed. The other two are unaffected. | L6 | Pair, Fault |
| LOSS-05 | Drop a link while the hub monitor shows one of its sessions. Type, and send a Kill, as it drops. | Every remote session of that relay becomes Disconnected and keeps its history read-only. It shows `Reconnecting...` while the hub monitor redials. Input typed during the gap is discarded and never replayed. The Kill is never replayed; its outcome is learned after the reconnect. | L8 | Monitor, Pair, Fault |
| LOSS-06 | Drop and restore a link while the hub monitor controls its selected remote session. Repeat with the session not controlled at the drop, with a local client taking control during the gap, and with another row selected. Repeat the first case with a local session whose daemon connection drops. | Only in the first case does the hub monitor take control again by itself. In the other three, and for the local session, recovery is passive and the placeholder shows. | L9 | Monitor, Pair, Fault |
| LOSS-07 | Lose a link and refuse every redial. Then let a link connect and die at once, and let one stay up 15 seconds. End links with each close reason. Hold a dial's TCP connect open. | The delay doubles from `monitor.limits.retry.initial_ms` to `cap_ms`, with the configured jitter, kept per relay, with no end. It resets only after a link has stayed up 15 seconds. After on hold, cut, busy, replaced, incompatible and protocol violation the hub monitor redials every 60 seconds with jitter. A connect not complete in 10 seconds is a failed dial. After "forgotten" and after its own user's disconnect it never dials. | K5-10, U18, U21-22 | Contract, Pair, Fault |
| LOSS-08 | While a relay waits to redial, press Reconnect, and edit its address. Start a monitor with several remembered relays, one refusing. Suspend the timer across a pending redial. | Reconnect and an address edit dial at once. A starting monitor dials every relay it was not told to disconnect, at once. The refusing relay returns to the 60-second pace after that one dial. A pending redial keeps the wait it had left; nothing treats sleep specially. | K11-13 | Contract, Monitor, Pair |
| LOSS-09 | With the on-path proxy holding the old connection open and silent, dial again from the same hub monitor. Drop a link while on hold is not set, then while several hub monitors are remembered. | The relay accepts the new connection in place of the old and closes the old with "superseded by your own new connection". It keeps listening, shows "Link lost, waiting for" the hub monitor, and never dials. A drop sets no hold, lifts none and forgets nobody. | U7, U19-20, W12 | Link, Pair, Fault |

## Authority and cut-off

| ID | Setup and action | Required observable result | Decisions | Layers |
|---|---|---|---|---|
| AUTH-01 | Start a relay with a remembered hub monitor and nobody at its terminal. | The hub monitor links with no prompt on the relay. Only a cut or a forget withdraws that consent. | U4-5 | Pair |
| AUTH-02 | While one hub monitor is linked, dial from a second remembered one, and from an unknown key. | The second is refused as busy, after it has authenticated. The unknown key learns nothing about whether anyone is linked. | U6 | Link, Pair, Fault |
| AUTH-03 | Cut a link. Dial from every remembered hub monitor. Lift the hold by accepting links, by a new pairing, and by stopping and starting the relay. Try to hold an idle relay. | Cut drops the link and puts the relay on [hold](../../../../GLOSSARY.md#monitor-links): every remembered hub monitor is refused and every pairing is kept. Each of the three actions lifts the hold. A relay the user starts always accepts links. The cut item exists only while a hub monitor is linked and reads "Accept links" on hold. | U8-9, RT11 | Relay, Pair |
| AUTH-04 | Close or refuse a link for each reason: cut, relay stopping, restarting for an upgrade, hub monitor forgotten, on hold, busy, replaced, hub monitor stopped. End one with no reason. | Each deliberate close is one Close record with its reason code and no text, sent after the handshake. A refusal sends Close alone, with no Hello from the relay. The hub monitor shows the reason against that relay's sessions. An ending with no reason is "link lost". | U16, W10, W12 | Link, Pair |
| AUTH-05 | Disconnect a relay from the hub monitor, restart the monitor, then reconnect. | The link closes and the relay shows "hub monitor disconnected". The hub monitor does not dial that relay, across restarts, until its user reconnects it. Both actions are immediate, with no confirmation. | U17, ST10, D12 | Monitor, Pair |
| AUTH-06 | Link, open streams and take control of some sessions. Watch the relay's screen, its terminal's bell and the sessions themselves. | The relay shows the hub monitor's name, fingerprint, source address marked unauthenticated, time linked, and which sessions it has open and controls. It never shows session content. A new link rings no bell, raises no notification and writes nothing into a session; a timestamped event line records it. | U13, U15 | Relay, Pair |

## Pairing store

| ID | Setup and action | Required observable result | Decisions | Layers |
|---|---|---|---|---|
| STO-01 | Pair, then read `links/` on each machine. Kill each program during a write. | The relay holds `relay-identity.json` and `hub-monitors.json`; the hub monitor holds `relays.json`, with the specified fields. Private keys are plain owner-only files. A write goes to a temporary file and is renamed, so an interrupted write leaves the old file or the new one. | ST2-5 | Contract, Pair, Fault |
| STO-02 | Damage each store file, and remove the relay's identity. Run every link command, the monitor and the relay. | A damaged file is an explicit error and never read as empty. Every link command fails with 125 and says why. A missing identity is created only on a relay's first start; with a new identity, every hub monitor sees a different relay and must pair again. | ST5, CM20 | Contract, Relay, Fault |
| STO-03 | Forget a hub monitor on the relay while it is linked, and while it is offline. | A live link closes at once with "hub monitor forgotten". An offline hub monitor gets "not paired" on its next dial. Either way it stops redialling and keeps its record, shown as `Pair again`. | ST8, W11 | Link, Pair |
| STO-04 | Forget a relay on the hub monitor while linked, while the relay is unreachable, and by command with no monitor running. | The hub monitor deletes its record and its keypair for that pairing. A live link closes with "forgotten by the hub monitor" and the relay drops its entry. Otherwise the relay keeps a stale entry that links nothing, until its user forgets it. | ST9, P18, CM16 | Monitor, Pair, Fault |
| STO-05 | Start a second monitor in the same home, open its links dialog, then exit the first monitor. | The first monitor holds a lock in `links/`. The second shows local sessions only and says another monitor holds the links. Its dialog lists the relays read-only, without live state, with every action disabled. It takes the links over when the first exits. | ST13, D14 | Monitor, Pair |
| STO-06 | With a monitor running, pair, forget and disconnect by command. Pair by command with no monitor running, then start one. | The running monitor notices each change: it dials the new relay and closes the forgotten or disconnected relay's link. A pairing command completes the first handshake, closes that link with "hub monitor disconnected" and leaves the record connected, so the next monitor dials it. | ST14-15 | Pair |
| STO-07 | List pairings on both sides. | Each side shows the other's name, fingerprint, pairing date and when it last linked. The hub monitor also shows the address and link state. The relay shows the name each hub monitor suggested and offers no rename. | ST7 | Monitor, Relay |

## Relay interface

| ID | Setup and action | Required observable result | Decisions | Layers |
|---|---|---|---|---|
| RLY-01 | Start the relay with `c2 relay` and `c2 r`, without a terminal, twice in one home, in two homes, and on a port already in use. Run `c2 exec`, the monitor and an install without starting one. Leave it with no sessions, link or pairing. | Both commands run `climon2-relay` from the CLI's own version. It refuses to start without a terminal. A second start in the same home fails and names the running relay; two homes run two relays. The listener is bound once at start and a bind failure is a start failure. Nothing but the command starts a relay, and an idle relay keeps running. | X1-2, X4, X7, X9-11 | Relay |
| RLY-02 | Stop the relay from its menu, with SIGTERM, and by closing its terminal, each while linked. Start it with an unknown flag and with each start failure. | Each stop is clean: the link closes with its reason and sessions keep running. The statuses are 0, 143 and 129. A usage error is 2. Any other start failure is 125, with the reason and the log file's name on stderr after the terminal is restored. A clean stop names no log. | X8, CM5, R3 | Relay, Pair |
| RLY-03 | Start the relay with no listen value, with `relay.listen`, with `--listen`, and with both. Give an IPv4 literal, an IPv6 literal, port 0, a host name and a missing port. | The default is `127.0.0.1:7222`. The flag overrides the setting, which is read once at start. IP literals with a port are accepted; a host name or a missing port is refused. With port 0 the screen shows the port actually bound. | CM1-4 | Contract, Relay |
| RLY-04 | Drive the relay through each state: listening, pairing, linked, link lost, on hold, incompatible and restarting. | The top row is a coloured status bar naming the state, with the specified detail on the row under it. Three panes are always visible: sessions served, remembered hub monitors, and events. Unlinked, the sessions pane says the relay is not looking at sessions. The bottom row lists one letter per action. | X3, RT1-9 | Relay |
| RLY-05 | Open pair, forget and stop, linked and unlinked. | Pair shows the PIN, the listen address as a hint, the time left and the attempts left. Forget lists the remembered hub monitors, asks "are you sure" and says what forgetting does. Stop asks for confirmation, defaulting to Cancel, only while a hub monitor is linked. | RT10, RT12-13 | Relay |
| RLY-06 | Look for any way to control or query a running relay from another terminal. | There is none: no control socket and no stop, status or forget command. The relay's screen is its only control surface. | X5 | Relay, Docs |

## Selector and links dialog

| ID | Setup and action | Required observable result | Decisions | Layers |
|---|---|---|---|---|
| SEL-01 | Show local sessions and sessions from two relays. Search by machine name. | One flat list in the existing order, with a machine column between status and title: `local`, or the relay's name. Search covers status, machine and title. | S1 | Monitor |
| SEL-02 | Bring each relay's link down for each reason. | Its rows read `Link lost`, `On hold`, `Upgrade` or `Disconnected`, with every close reason mapped onto one of them. They sort where Disconnected rows sort. Final rows keep their outcome. | S2 | Monitor, Fault |
| SEL-03 | Highlight a row whose link is down, then one whose link is up. | A status strip above the footer shows the relay's name, the reason and what happens next. Otherwise it shows one state mark per relay. | S3 | Monitor, Fault |
| SEL-04 | Select a row whose link is down, while the hub monitor is redialling and while it is not. | Its retained history opens read-only and the chrome names the relay and the reason. The `Reconnecting...` overlay shows only while redialling; otherwise an overlay names the reason. | S4 | Monitor, Fault |
| SEL-05 | Pair with a relay, keep its suggested name, then rename it. | The name defaults to the relay's hostname, which arrives inside the encrypted link. The user's chosen name never crosses the link. | S5 | Link, Monitor |
| SEL-06 | Look for link actions in the selector. | The selector gains none. Pair, name, disconnect, reconnect and forget live in the links dialog and in commands. | S6 | Monitor |
| DLG-01 | Press prefix+l over a session and over the selector, with relays in each state and with none. Open help. | The dialog opens with one row per relay: name, state, release version, address and when it last linked. The states are Linked, Link lost, On hold, Upgrade, Disconnected and Pair again. A detail strip gives the highlighted relay's fingerprint, pairing date, and the reason and next step when its link is down. With no relays it explains how to get a PIN. Help lists the binding. Escape closes it. | D1-5 | Monitor |
| DLG-02 | Edit a relay's name and address. Forget one. Give a relay another's name. | One form changes the name and the address, and no change needs a PIN. A name already in use is refused. Forget asks for confirmation, defaulting to Cancel, and says what it does. | D9, D11, D13 | Monitor |
| DLG-03 | Do each dialog action by command instead. | Each command has the same effect as the dialog action, including the unique-name rule. | D15 | Contract, Pair |

## Commands and configuration

| ID | Setup and action | Required observable result | Decisions | Layers |
|---|---|---|---|---|
| CMD-01 | Run each `c2 monitor` verb by its full name and one-letter shortcut, bare `c2 monitor`, and an unknown verb. | Each verb works under both spellings. The bare command still starts the monitor. An unknown verb is a usage error with status 2. | CM6 | Contract, Pair |
| CMD-02 | Run `edit` with a name, an address, both and neither. Name a relay by exact name, by fingerprint prefix, ambiguously, and where one relay's name is another's fingerprint prefix. Use a host name as an address and change what it resolves to. | `edit` needs at least one flag. A relay is its exact name or an unambiguous fingerprint prefix, and the name wins a clash. No match, or several, fails with 125 and lists the matches. A host name is resolved at each dial. | CM7-9 | Contract, Pair |
| CMD-03 | Run `edit`, `disconnect`, `reconnect` and `forget` with and without a monitor running, including when nothing would change, and `reconnect` on a relay showing `Pair again`. | Each returns once the store is written, dials nothing and prints nothing. A command that changes nothing exits 0. `forget` asks nothing. `reconnect` on `Pair again` fails with 125 and says to pair. | CM16-19 | Contract, Pair |
| CMD-04 | Run `c2 monitor list` and `list --json` with a monitor holding the links, with none, and with a stale status file left behind. | The columns are NAME, STATE, VERSION, ADDRESS, LAST LINKED and FINGERPRINT; JSON adds the pairing date and the reason a link is down. Live state comes from an owner-only status file, trusted only while a monitor holds the links lock. Otherwise STATE shows what the store knows and the rest show `-`. | CM21-23 | Contract, Pair |

## Records and secrets

| ID | Setup and action | Required observable result | Decisions | Layers |
|---|---|---|---|---|
| LOG-01 | Run a relay, a hub monitor and each link command, then read the logs. | The relay writes one general log per run under a `relay` role. The hub monitor's link records are in its monitor log and each command's in the CLI's general log. The existing level, size cap and seven-day deletion apply unchanged. | R1-2 | Relay, Pair |
| LOG-02 | Produce more than 200 events on a relay across midnight, then restart it. | Every events-pane line is also an INFO record. The pane keeps the last 200 lines, starts empty on every start, and shows local `HH:MM:SS`, with the date on lines from an earlier day. The log stays UTC. | R4-5 | Relay |
| LOG-03 | Do everything a relay records: start, stop, each invitation outcome, a failed attempt, link up and ended, each refusal, cut, accept, forget, a violation, an upgrade restart. Open, control and kill sessions from the hub monitor. | Each has its INFO record with the specified fields. Session activity is recorded by session id only: streams opened and closed, control gained and lost, and Kill requests. | R6-7 | Relay, Pair |
| LOG-04 | Flood the relay with connections that fail before authentication. Redial repeatedly from a refused hub monitor, and repeat one violation. | Unauthenticated failures are counted, with at most one summary line a minute and individual lines only at DEBUG. A repeated refusal or violation is recorded once, then counted until the reason changes or that hub monitor links. Failed pairing attempts stay individual. | R8-9 | Relay, Fault |
| LOG-05 | Start a relay, end a link, run it idle past 24 hours, and fill its log to the cap. | The effective `relay.limits` and release constants are logged once at start. Each ended link leaves one line of counters. A "still running" record is written every 24 hours. Reaching the cap is said once in the events pane. The screen shows neither limits nor counters. | R10-13 | Relay, Fault |
| LOG-06 | On the hub monitor, pair, link after several failed dials, lose the link, and use each link action. | INFO records cover pairing results, link up with the number of dials, link ended with its reason, the first failed dial after a loss, each action and each violation. Later failed dials appear only at DEBUG. Each ended link leaves one line of counters. | R14-15 | Monitor, Pair |
| LOG-07 | Link with stand-ins whose suggested names hold control characters, escape sequences and more than the fixed length. | Each name is logged and shown with control characters removed and cut to the fixed length. Arguments, working directories, environment values, session content and copied text appear in no log at any level. | R16, R18 | Monitor, Relay, Fault |

## Hostile relay

A compromised relay may lie about its own sessions and nothing else. These
scenarios run a hub monitor against a link peer stand-in in the relay's role
and check the hub monitor's host terminal with the terminal oracle.

| ID | Setup and action | Required observable result | Decisions | Layers |
|---|---|---|---|---|
| HOS-01 | Send escape sequences and control characters in the relay's name, session titles, commands, arguments and working directories. | They reach the selector, the links dialog, toasts and logs as inert text. The host terminal receives none of them as sequences. | T9, R18 | Monitor, Fault |
| HOS-02 | Break the protocol on one link while the hub monitor holds other links and local sessions. | That link closes with its reason. The monitor keeps running and its other links and local sessions are unaffected. | T9, LM9 | Monitor, Fault |
| HOS-03 | Send copy records, repeatedly and at every size. | Nothing reaches the host clipboard until the user presses the consent key, and the copied text is never drawn. | T9, C11 | Monitor, Fault |
| HOS-04 | Advertise session ids that match another relay's sessions and local sessions, and send records for streams the hub monitor never opened. | No session of another relay, and no local session, changes row, content, status or control. | T9, A | Monitor, Fault |
| HOS-05 | Feed arbitrary bytes to every decoder in the link code, from a recorded seed and for a bounded number of cases, in the ordinary test run. | No decoder panics, reads past its input or allocates beyond its stated bound. Coverage-guided fuzzing is not a gate. | T9, T5 | Link |

## One link implementation

The map requires the code that speaks between a relay and a hub monitor to be
the same on every platform, so that evidence from one operating system holds
for every pairing of them. A **platform seam** is the interface behind which
climon2 keeps code that differs by operating system
([ADR 0003](../../../adr/0003-platform-and-shell-seams.md)).

| ID | Setup and action | Required observable result | Layers |
|---|---|---|---|
| ONE-01 | Run the workspace test that reads `cargo metadata` ([ADR 0004](../../../adr/0004-crate-layout-and-dependency-direction.md)), extended for `climon2-link`. Run its checker against sample crates that each break one rule. | The test fails when `climon2-link` depends on anything outside its allowlist, which holds no `climon2-platform`, no operating-system or socket crate and no async runtime. It fails on any `cfg` that names an operating system, family or architecture, and on any use of `std::net`, `std::os`, `std::fs`, `std::process`, `std::env` or the standard clocks. Each sample crate is rejected for its own rule. | Contract |
| ONE-02 | Inspect how the link code obtains time and randomness. | Both arrive only as inputs from the caller. ONE-01 rejects any other source. | Contract, Link |
| ONE-03 | Run one shared contract suite per seam (sockets, key storage, randomness and clock) against each platform's implementation, in that platform's own test job. | Every implementation passes the same suite, with no link code and no second machine. Key files are owner-only, writes are atomic and a damaged file is an error. A listener honours the connect and opening deadlines. Randomness comes from the operating system's generator. | Contract |
| ONE-04 | Run the Link layer and replay every fixture in every platform's test job, including Windows. | The same tests pass on every platform with no platform-specific expectation. | Link |

`climon2-link` is not required to build without the standard library. That
would be a stronger proof, but it would constrain the choice of cryptographic
crates, which is left to the implementation.

### What Windows runs

Monitor links on Windows are outside the first release. From that release the
Windows test job still runs the whole Link layer and replays every fixture,
which proves the link code is the same there. Seam suites, Relay and Pair
scenarios on Windows, and content fixtures recorded on Windows, wait until
monitor links on Windows are specified.

## Fixtures

Recorded fixtures are committed to the repository and replayed by every
platform's test run, on both ends. They let each pairing of operating systems
be checked against the same bytes without the two ever meeting.

### Protocol transcripts

A [protocol transcript](../../../../GLOSSARY.md#testing) holds the complete
bytes of one exchange, both directions, made with fixed test keys and fixed
randomness. Link revision 1 has these:

| Transcript | Exchange |
|---|---|
| Opening | An [opening request](../../protocol/monitor-link/opening-request.md) and [reply](../../protocol/monitor-link/opening-reply.md) that choose revision 1, and a refusal with no shared revision. |
| Pairing | The four pairing messages ([shares](../../protocol/monitor-link/pairing-share.md) and [confirmations](../../protocol/monitor-link/pairing-confirmation.md)) for a correct PIN, and a run that fails on a wrong PIN. |
| Handshake | The pinned-key handshake ([initiation](../../protocol/monitor-link/handshake-initiation.md) and [reply](../../protocol/monitor-link/handshake-reply.md)) and both [Hello](../../protocol/monitor-link/hello.md) records. |
| Refusal | A handshake answered by [Close](../../protocol/monitor-link/close.md) alone, once for each refusal reason. |
| Link | A short link: a snapshot, one stream opened, requests, events split into fragments, [credit](../../protocol/monitor-link/credit.md), a [keepalive](../../protocol/monitor-link/keepalive.md), a stream ending and a Close. |

Replay gives one end the recorded bytes of its peer and requires it to emit
its own recorded bytes exactly, then does the same for the other end.

Once a link revision ships, its transcripts are never refreshed. A changed
byte is a failure, not a reason to record again. A new link revision adds
transcripts of its own and keeps the old ones.

### Content fixtures

A [content fixture](../../../../GLOSSARY.md#testing) holds two things recorded
from one scripted real session: the client interface events the session
produced, and the link records they become, taken before encryption.
Encryption does not depend on content, so these fixtures stay readable and
need no keys.

Each script is recorded on GNU/Linux and on macOS:

| Script | Content |
|---|---|
| Paths and titles | The [generic terminal fixture](../../../../GLOSSARY.md#testing) with non-ASCII and non-UTF-8 paths, arguments and titles. |
| Full-screen program | The [TUI fixture](../../../../GLOSSARY.md#testing): alternate screen, mouse, paste and a redraw after resize. |
| Reports | Status, progress and notification reports. |
| Clipboard | A clipboard copy. |
| Ending | A session that ends, with its run report. |

Replay on the relay's side feeds the events to the encoder and requires the
recorded records. Replay on the hub monitor's side feeds the records to the
monitor and checks the terminal oracle against expectations written by hand,
not derived from the recording.

A content fixture is recorded only by a deliberate capture command, never as
a side effect of a test run. Recording is additive: a new script or a new
operating system adds files, and existing files are kept. Because a fixture
holds the client interface events as well as the link records, content from
Windows can later be recorded from a Windows session without a Windows relay.

## Documentation

| ID | Setup and action | Required observable result | Decisions | Layers |
|---|---|---|---|---|
| DOC-01 | Review the published documents against the decisions that are promises to the reader and not behaviour. | They say that traffic analysis is not defended and that an administrator is outside the threat model. They record the cryptographic crates as pinned and unaudited, an accepted risk. They say that `links/` holds secrets and that copying a home clones its keys. | T4, T7, T12, ST6 | Docs |

## Decisions with no scenario

| Decision | Why |
|---|---|
| T1 | It scopes the threat model to macOS and Linux and promises no behaviour. |
| L7 | `c2 exec --keep` and `--detached` are specified and built outside this map, by [Add c2 exec --keep and --detached for sessions that outlive their clients](https://github.com/jackgeek/climon2/issues/423). |
| ST11 | Withdrawn by RT14, which REV-08 and AUTH-03 cover. |
| ST16 | A glossary definition. |
| C1-10, C12 | Clipboard copies for local sessions are specified and accepted outside this map, by [Offer a session's clipboard copies in the monitor (clipboard-v1)](https://github.com/jackgeek/climon2/issues/476). |

## Missing capabilities

Each of the following must be built before the scenarios that need it can
run. Struck through are those that exist:

- ~~the Link layer's harness, which supplies time and randomness~~ (#492);
- ~~the on-path proxy and the link peer stand-in~~ (#501, test-only, in
  `climon2-e2e`'s test support);
- ~~a harness that runs two climon2 homes on one host and joins them over
  loopback~~ (#501, likewise);
- the test-build clock hook for the relay, and the same for the monitor's
  redial timer;
- the capture command for content fixtures;
- ~~the extension of the `cargo metadata` workspace test~~ (#492).

The monitor's
[existing fixtures](../monitor/acceptance.md#existing-fixtures-and-missing-capabilities)
are the starting point for the Relay and Pair layers: the terminal harness,
per-test isolation and the packaged-test route.

## Deferred scope

No gate here covers monitor links on Windows beyond
[what Windows runs](#what-windows-runs), a relay with no terminal, the relay's
upgrade restart on Windows, or rendezvous plugins. Relay chaining and more
than one hub monitor per relay at a time are outside the specification.
