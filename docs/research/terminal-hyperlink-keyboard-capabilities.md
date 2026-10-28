# OSC 8 and extended keyboards: model contract versus host facts

Research date: 2026-10-28. Resolves the evidence objective of [OSC 8 and extended keyboard capability contracts across supported terminals](https://github.com/jackgeek/climon2/issues/54), under [Wayfinder: session continuity, attachment, and scrollback release](https://github.com/jackgeek/climon2/issues/40). It does **not** resolve [Daemon virtual terminal: engine, ownership, and failure boundaries](https://github.com/jackgeek/climon2/issues/43).

## Result and boundaries

**OSC 8 and both keyboard protocols are requirements, not exclusions. The pinned Alacritty library alone does not meet that contract.** Its parser recognizes modifyOtherKeys but its `Term` supplies no implementation; Kitty direct-set state and query state diverge. iTerm2 and VS Code have real native-event encoder paths, not just CSI parsers, but neither a flag echo nor a version string establishes full input fidelity. Terminal.app's extended protocol support remains unqualified, not proven impossible.

Inherited decisions are unchanged: in-process Alacritty family behind platform-neutral, pure/I/O-free `climon2-vt`; one session-daemon query authority; derived screen output, not raw application suffixes; no client emulator; independent output oracle. The command sees `TERM=xterm-256color`, `COLORTERM=truecolor`, not inherited host identity. Host facts may travel through the controlling client; the daemon answers once. Missing facts are unsupported, never invented; intentionally detached sessions retain last-known facts. This report chooses neither rejection nor downgrade, no new takeover/admission policy, and no fabricated releases/repeats. Existing geometry, resource, history, lifecycle, hand-back and live-title/bell decisions remain intact. No product implementation or general vulnerability audit was performed.

This adds only protocol/input evidence to [Alacritty embedding: state access, history identity, and enforceable budgets](https://github.com/jackgeek/climon2/issues/53). Native-history research is not repeated.

## Exact evidence targets

| Target | Version/revision used |
|---|---|
| Model; Alacritty frontend only where explicitly distinguished | `alacritty_terminal` **0.26.1-dev**, **bbb2cbf9470a6707b1205fbddbdb25109b14e2fd**; VTE **0.15.0**, source tag commit **3b3da71c34cc1256c7e20981cf03f8eb95e08ffc**. Probe explicitly pins VTE `=0.15.0`. [Manifest][a-manifest], [parser][vte] |
| Terminal.app | **2.15 (470.2)**, identity reused from [Native scrollback restoration without clearing unrelated terminal history](https://github.com/jackgeek/climon2/issues/51). Apple's current keyboard guide links to Terminal 2.15/macOS 27. No public emulator implementation or first-party extended-keyboard/OSC8 guarantee was established in the inspected guide. [Apple][apple] |
| iTerm2 | Stable **3.7.6**, tag `v3.7.6` resolves to **7ce456b98db38d3f0bafd70ffec87752e69d5d58**, reverified through GitHub. All iTerm source links below use this commit, not development HEAD. [Tag][i-tag] |
| VS Code integrated terminal | **1.143.1**; its release lockfile resolves `@xterm/xterm` **6.1.0-beta.304**; npm `gitHead` **c58ea3637f3968e0e6e79cd92cf9aace7ef89ee2**, both reverified. Host adapter/configuration links use release `1.143.1`, not VS Code HEAD or standalone xterm's latest release. [Lock][vs-lock], [package][x-package] |
| Protocol definitions | Kitty current first-party specification; OSC8 author's proposal; xterm **patch 411**, including tagged source for the query reply that its reference abbreviates. These define semantics, not support in other hosts. [Kitty][kitty], [OSC8][osc8], [xterm reference][ctlseq], [xterm source][xt-query] |

**D** below means documented behavior; **S** means inspected source; **P** means executed isolated source probe; **U** means unexecuted native-host qualification. None of the native hosts was launched, controlled, queried or sent synthetic input.

## Compact capability matrix

| Target | Normal input, press/repeat/release and disambiguation | Kitty mode/query contract | modifyOtherKeys (MOK) | OSC8 ID/URI |
|---|---|---|---|---|
| **Alacritty model at pin** | **S:** `Term` is an output/state model, **not an input event decoder or key encoder**. `TermMode` exposes effective Kitty bits. Alacritty's **separate GUI frontend** sends text/C0/legacy keys normally; uses `KeyEvent.repeat` and release events when enabled. Its encoder omits base-layout alternate keys and lock modifier bits. That GUI code is not an embedding API. [Modes][a-term], [frontend][a-key] | **S/P:** all five defined flags; default library `Config.kitty_keyboard=false` (GUI explicitly enables it). Normal/alternate stacks, 4,096 entries; RIS clears both. **Direct `=flags` changes effective mode but not the stack queried by `?u`**. Unknown bits truncated by VTE. [State][a-term], [GUI config][a-ui], [dispatch][vte], probe below | **S/P gap:** VTE calls `set_modify_other_keys` / `report_modify_other_keys`, but their trait defaults are no-ops and `Term` overrides neither. No MOK state, effective input behavior or reply is furnished by this embedding. [Trait/dispatch][vte], [Term][a-term] | **S:** parses URI and optional `id`, rejoins semicolons in URI; stores metadata on cell template/cells. Missing ID gets generated suffix `_alacritty`; no local ID/URI length validation in these handlers. [Parser][vte], [cell][a-cell], [setter][a-term] |
| **Terminal.app 2.15 (470.2)** | **D:** editable key mappings and Option-as-Meta. **U:** exact default bytes, aliases, auto-repeat treatment, release reporting, alternate/base-layout data and IME behavior in this build. Ordinary legacy UTF-8/C0/CSI input is the baseline to qualify, **not a measured byte contract here**. [Guide][apple] | **U:** no established first-party Kitty support/query/stack guarantee in the inspected surface. Do not mark supported from `TERM=xterm-256color`, a DA reply, auto-detected URLs, or another host's support | **U:** no established MOK level/query guarantee. Do not silently classify a missing reply as proof of all possible native behavior | **U:** no established OSC8 ID/URI guarantee or length limit. Recognizing literal URLs would not prove OSC8 arbitrary-label metadata |
| **iTerm2 3.7.6** | **S:** native `NSEvent` keyDown, `isARepeat`, keyUp and modifier-change paths; normal text/legacy fallback; Kitty press/repeat/release `1/2/3`, shifted/base-layout and associated-text paths. Dead-key keyUp is deliberately suppressed. Option settings, bindings and OS-consumed keys still matter. **U** actual physical input. [Mapper][i-modern], [settings D][i-keys] | **S:** set/OR/clear, push/pop and current-state query implemented; **unmasked requested integers can be echoed**, not a supported-bit set. Profile “Apps can change how keys are reported” gates mapper changes separately from parser state. Normal/alternate stacks; source pruning at count 1,024 removes 1,024 entries (at that boundary, empties the stack), not standard oldest-one eviction. [Dispatcher/stack][i-term], [activation][i-session] | **S:** levels 1 and 2 have actual distinct mappers; default profile allows application changes. `CSI >4;N m` changes state and clears current Kitty stack/base flags; `CSI >4 n` disables. **No `CSI ?4m` query dispatch found** in this release's exhaustive CSI switch; successful setting does not imply a query exists. No release events in MOK. [Parser][i-csi], [state][i-term], [mappers][i-mok1], [encoder][i-mok], [defaults][i-profile] | **D/S:** explicit URI + ID; same URI/different IDs separate hover anchors. URI parser uses `NSURL`; configurable default limit is **2,097,152 NSString units**, not the proposal's historical 2,083-byte figure. No ID-specific limit found in this path. **U** 8,192-byte target/256-byte ID native roundtrip/hover. [Docs][i-esc], [handler][i-term], [limit][i-advanced] |
| **VS Code 1.143.1 / packaged xterm** | **S/P:** real DOM keydown `repeat` and keyup paths, conditional Kitty encoder; legacy text/C0/CSI otherwise. **Partial fidelity:** no base-layout alternate output; modifier encoder only Shift/Alt/Ctrl/Super, not lock bits. At flags **31**, ordinary `a` press and repeat both encode `CSI 97;;97u`: repeat identity is lost when associated text is present. Native chords/editor shortcuts can intercept events before xterm. **U** Electron/native delivery. [Service][x-service], [encoder][x-key], [DOM routing][x-core], [VS routing][vs-instance] | **S/P:** host enables `vtExtensions.kittyKeyboard` from **`terminal.integrated.enableKittyKeyboardProtocol`, default true**. All set/query/push/pop handlers are gated by it. Query echoes stored flags, including unknown **32**; 16 saved entries per screen, separate main/alt flags, reset to zero. Pop-to-empty resets zero even if a direct-set base had been saved. [Host config][vs-config], [adapter][vs-xterm], [handlers][x-input], [reset][x-state] | **S:** no MOK setter/query/state path in inspected InputHandler registrations, normal encoder or keyboard service; a few CSI-u encodings do **not** establish MOK negotiation. This packaged release has Kitty support, **not thereby MOK**. [Handlers][x-input], [normal encoder][x-legacy], [service][x-service] | **S:** parser stores optional ID + URI; service keys explicit links by `(id,URI)`. **Actual VS Code host** installs `linkHandler`, allows non-HTTP protocols through to its scheme policy and modifier-gated opener. Not just standalone-engine support. Native user action/permissions/scheme policy remains host-owned. [Parser][x-input], [identity][x-links], [host link integration][vs-links] |

## Wire semantics that the virtual terminal must own

`CSI` = `ESC [`; `OSC` = `ESC ]`; use constructed 7-bit `ST` = `ESC \` when rendering OSC8. Spaces in notation are not transmitted.

### Kitty: flags are progressive enhancements, not a feature inventory

[The specification][kitty] defines:

| Value | Meaning / important exception |
|---|---|
| **1** | Disambiguate Escape and modified keys. Ordinary text remains UTF-8; Enter, Tab and Backspace retain legacy bytes unless other requested enhancements require encoding |
| **2** | Event types: press **1** (default/omittable), repeat **2**, release **3**, in modifier subparameter, e.g. `CSI 97;1:3u`. Without this, repeat is indistinguishable from another press; no release report |
| **4** | Alternate keys: `key:shifted:base-layout`; only enhances events already encoded. The presence of this bit is not proof the OS/encoder supplies both alternatives |
| **8** | All keys as escapes, including text-producing and modifier keys; needed with **2** for Enter/Tab/Backspace releases. Plain key text is replaced by event encoding |
| **16** | Associated text as Unicode codepoints; defined in conjunction with **8**, not an independent arbitrary-text event source |

Encoding is `CSI key[:shifted[:base]];modifiers[:event][;text[:text...]]u`, with functional-key `~`/letter forms also retained. Modifiers are **1 + bitmask**: Shift 1, Alt 2, Ctrl 4, Super 8, Hyper 16, Meta 32, CapsLock 64, NumLock 128. IME/text-only events need their actual observed text, not an invented physical key; the spec permits key code zero when key identity is unknown.

`CSI =F;M u`: replace (**M=1**, default), OR (**2**), clear selected bits (**3**). `CSI ?u` replies `CSI ?F u`: **current active flags**, not maximum supported flags or a list of working native-event fields. `CSI >F u` pushes/activates F (default 0); `CSI <N u` pops N (default 1), empty restores zero; main/alternate stacks are separate, bounded, and specification overflow evicts oldest. `CSI =0u` clears current flags; it is **not** “pop every saved mode.” Reset/restore and stack semantics must be checked independently on each target.

Kitty suggests query then DA1 as an ordering sentinel, and set-then-query to detect partial implementations. That is useful **only where the implementation reports effective supported state**. Here:

- **Alacritty:** the query reads `keyboard_mode_stack.last()`, whereas direct set changes `TermMode` only. Even an honest query can report **0** while all five effective flags are set. Screen swap reloads from stack too. VTE masks unknown bits to **31**, but that mask says nothing about native event availability. [Source][a-term], [dispatch][vte]
- **iTerm2:** integer assignment/query does not mask unknown flags; profile input activation is separate. Query success is parser/state evidence, not proof application requests changed the event mapper or every key reaches it. [Source][i-term], [activation][i-session]
- **VS Code:** handler accepts/echoes **63**, including undefined bit **32**, while the encoder implements only named behaviors and has the demonstrated repeat/associated-text gap. Disabling the host setting suppresses query replies as well as activation. [Handlers][x-input], [encoder][x-key], [host setting][vs-config]

Thus neither “echoed requested bits” nor `CSI >q`/DA2/proprietary version replies are a supported-event capability bitmap. Versions identify a candidate for a qualification profile; settings, layout, OS shortcuts, focus and host adapters still affect delivery.

Reset distinctions also matter: iTerm's RIS and DECSTR reach `commonReset`, clearing flags/both stacks and resetting send-modifier resources to **-1**; xterm's DECSTR calls `CoreService.reset`, clearing Kitty state, and RIS invokes full terminal reset. Alacritty's RIS clears both stacks/effective modes, but this VTE dispatch has no DECSTR handler. These are source facts, not permission to issue a destructive terminal reset just to clean up keyboard modes. [iTerm][i-term], [xterm handlers][x-input], [xterm core][x-core], [VTE][vte]

### modifyOtherKeys: level query is current state, not the highest supported level

[xterm][ctlseq] uses `CSI >4;L m`: **0** disables MOK behavior, **1** enables modified other keys except well-known legacy cases, **2** includes more modified keys. Unmodified text is still text; functional-key modes are separate. Default output is `CSI 27;modifier;codepoint~`; `formatOtherKeys` can select CSI-u formatting instead. This changes format, **not** press/repeat/release capability. In legacy/MOK input, repeats are more occurrences of the same encoded press and releases are absent. Level 1 does not make every legacy alias distinguishable; neither level recovers events intercepted before the terminal receives them. [Semantics][modified], [level definitions][vte]

`CSI ?4m` asks for the **current** `modifyOtherKeys` resource, replying `CSI >4;L m` in actual [xterm patch-411 source][xt-query]. The reference abbreviates the reply; the source explicitly sends two parameters. A reply with L=0 means implemented query/currently zero, **not “MOK unsupported.”** There is no maximum-level capability bitmap in this query; a timeout alone is not a negative capability proof. Encoding format has a separate query, `CSI ?4g` (XTQFMTKEYS). `CSI >4m` resets to the configured initial resource value, not necessarily zero; `CSI >4n` disables the resource (xterm resource value -1). MOK has no Kitty-style push/pop stack; save/query/restore and Kitty coexistence require an explicit contract.

In the pinned VTE, missing MOK level defaults to zero rather than xterm's configurable initial value, and only explicit levels 0/1/2 plus `?4m` callbacks exist. In the pinned **Alacritty Term those callbacks do nothing**. Implementing just a response would falsely advertise a mode whose behavior is still missing. iTerm2's MOK setting changes clear Kitty state; blindly replaying both protocols' controls is not a portable nesting strategy. [VTE][vte], [Term][a-term], [iTerm state][i-term]

### OSC8: retained metadata, not output replay or network activity

[OSC8][osc8] sets a cell attribute: `OSC 8;id=ID;URI ST`; close with `OSC 8;;ST`. Switching directly to another link is legal; SGR reset is not its close operation. `(ID,URI)` groups an anchor; one ID with different URIs is not one target. Absent IDs have host-specific grouping heuristics. The proposal recommends stable, collision-separated IDs for multiplexers/restoration. Preserve explicit IDs and distinguish generated IDs internally; do not casually regenerate an ID per painted fragment and split a single logical anchor. Alacritty's generated `_alacritty` IDs become part of the adapter's identity/mapping concern, not proof that the original command supplied an ID. [Cell construction][a-cell]

The agreed limits stay **URI ≤8,192 bytes**, optional **ID ≤256 bytes**; invalid/control-bearing/over-limit values cause explicit failure, never silent truncation. The existing **64 KiB control-string / 4 KiB title** ceilings still apply. The model's cell setter is not the pre-parser admission boundary. Validate URI syntax/encoding and delimiters before constructing OSC; the proposal's portable wire form is printable ASCII **32–126**, URI-encoding non-ASCII, and ID parameter values cannot contain raw `:` or `;`. A mapping/namespace must respect the 256-byte emitted-ID limit; do not silently expand an otherwise maximum-length ID beyond it. Exact validation/mapping grammar belongs to the profile decision, not a change made here.

Reconstructing validated OSC8 during derived live output/restore is allowed. Explicitly close links at rendering boundaries so metadata does not leak to unrelated later text. No original OSC payload replay, fetch, automatic opening, clipboard/proprietary notification/file/image expansion, or historical title/bell emission is implied. Host click-to-open behavior is independent of climon2 automatically opening anything.

**There is no portable OSC8 support/ID/URI-length query in the proposal.** There is, however, a real **iTerm-specific plain-PTY exception**: `OSC 1337;Capabilities ST` is handled under trust/report gates; replies `OSC 1337;Capabilities=... ST`, with **H** advertising hyperlinks. The release constructs H=true. That advertises the host feature, **not** length, scheme, grouping or input-event fidelity; absent reply remains unknown. This does not require Python/AppleScript integration and is not portable to the other hosts. [Dispatch][i-term], [report/H encoding][i-output], [capability construction][i-session]

Do not reuse the OSC8 proposal's old “iTerm 2,083-byte limit” as this release's limit: current pinned source defaults to **2,097,152 NSString UTF-16 units**. That is source evidence, not a guarantee of accepted URI schemes or every 8,192-byte payload under every setting. Terminal.app lacks equivalent established evidence here; the required hyperlink contract remains a real host-qualification gap.

## What a client can actually observe

| Plain PTY observation | What it establishes — and does not |
|---|---|
| Correlated Kitty query response, optionally bracketed by a known DA1 response | A responder on the terminal path implements that response/current state. Account for the exact-source caveats above. No guaranteed native key coverage, authenticated host identity, supported-bits bitmap or arbitrary timeout proof |
| Correlated MOK level/format reply, **if implemented** | Current level/format, not max level, all-key disambiguation or release capability. iTerm's setter without query is a concrete exception to “set implies query” |
| Actual received input bytes | The emitted event/text **for that observation**. Kitty can expose repeat/release/modifiers when provided; legacy byte `09` cannot say Tab versus Ctrl+I, `0d` Enter versus Ctrl+M, `1b` Escape versus a prefix. A decoder cannot recover information never transmitted, regardless of timeout heuristics |
| iTerm Capabilities H / host version queries | Host-specific advertisement/identity. Querying over a PTY is possible without GUI integration; advertisement remains narrower than full native qualification |
| `TERM`, `TERM_PROGRAM`, `TERM_PROGRAM_VERSION`, `COLORTERM`, terminfo | Static environment/database hints, possibly inherited through ssh/nesting, **not observation**. Replacing command identity with xterm/truecolor neither upgrades the host nor makes these hints authoritative |
| No incoming hyperlink reply | No portable observation of OSC8 rendering, hover grouping, click behavior, accepted max ID/URI or scheme policy. Native readback/host API or qualification is needed; query absence is not universal impossibility |

The daemon cannot query an absent outer terminal. A controlling client can send an approved, bounded query, demultiplex its expected reply and return **typed host facts**, tagged with attachment/control-period and probe generation. It must not pass those replies as ordinary keyboard input or forward application queries for a second host answer. The daemon answers the command once using authoritative model state and qualified host facts. Client input decoding/probe correlation is **not** an emulator, but arbitrary CSI-u bytes, paste and user input can resemble replies: correlation is not authentication. Query/reply framing, deadlines and stale-reply handling still belong to the existing protocol decision.

Opt-in iTerm Python/native APIs or a VS Code extension can expose host settings/event context not in ordinary PTY bytes; they are extra integrations/permissions, not capabilities obtained by reading TERM. They cannot promise OS-intercepted keys that never reach the host. This research did not invoke them.

## Actionable options and remaining human choices

These are alternatives/evidence implications, **not an implementation plan or selected policy**:

1. **Narrow model extension/adapter:** add explicit MOK state/set/query/reset and input encoding semantics; reconcile Kitty effective flags with query/stack/screen/reset state. Observe both keyboard protocols' semantic transitions even when screen cells do not change. Expose a protocol-neutral, I/O-free keyboard contract behind `climon2-vt`; the current GUI-only `build_sequence` is not that interface. Source patches/vendor are allowed but none implemented here.
2. **Native-byte acquisition plus daemon translation:** request a separately qualified outer input mode, decode the actual bytes, and translate to the command's current virtual protocol. For example, a qualified Kitty event can produce a requested MOK press encoding; an observed MOK/legacy press cannot become a truthful Kitty release, repeat label or missing alternate-layout key. Translation requires layout/text/modifier provenance, not simply relabeling all CSI-u numbers.
3. **Host profiles/integration:** qualify exact releases/settings and a precise subset of event fields, with optional integrations where required. `query_present`, `requested_flags`, `observed_event_fields`, OSC8 rendering and known limitations must not collapse into one `supports_kitty` Boolean. Profiles cannot assert that the pinned VS Code repeat/associated-text path is lossless.

The existing live decision retains the choice of rejection versus an explicitly agreed downgrade, what happens when capabilities change during immediate takeover/most-recent-controller hand-back, and the frame schema/exact advertised keyboard profile. No blanket exclusion satisfies the current feature requirement. **Newly sharpened within those existing questions:** whether the required profile includes base-layout/lock-state fidelity and text-associated repeats (the pinned sources demonstrably lack some), and how emitted hyperlink ID namespacing fits the fixed 256-byte boundary. No new ticket or ADR amendment is made.

## Executed evidence and independent qualification oracles

**Performed:** pinned public source/docs inspection; release dependency/tag verification; six real Alacritty/VTE parser-to-`Term` assertion cases (12 reply/mode assertions), and eight isolated pinned xterm handler/encoder assertions. Automated document/source checks verified **37** used/resolved citation definitions, **28** pinned GitHub source URLs/line ranges, and **11** source/pin assertions (including byte equality of the probe's VTE registry source with the cited VTE commit). No live terminal, GUI, automation event, clipboard, link opening or network target fetch from a hyperlink was used. Fetching public primary documentation/source is separate. No product tests/builds are represented as run.

Alacritty probe: Rust 1.96.0, `default-features=false`, library from the exact source archive, VTE `=0.15.0`, `Config { kitty_keyboard: true, ..Default::default() }`, 80×24 `Dimensions`, `EventListener` recording only `Event::PtyWrite`, fresh `Term`/`Processor` per row. It asserted both the complete reply list and effective `REPORT_ALL_KEYS_AS_ESC`:

| Bytes fed (CSI notation) | Only reply | Effective report-all |
|---|---|---|
| `=31u`, `?u` | `?0u` | true |
| `>31u`, `?u` | `?31u` | true |
| `>1u`, `=31u`, `?u` | `?1u` | true |
| `>63u`, `?u` | `?31u` | true |
| `>31u`, `<u`, `?u` | `?0u` | false |
| `>4;2m`, `?4m`, `?u` | `?0u` (**no MOK reply**) | false |

xterm probe: Node **24.18.1** `node:module.stripTypeScriptTypes(...,{mode:'transform'})` on the **unmodified pinned Kitty encoder** and its source enum dependencies; exact four Kitty handler methods extracted from InputHandler into an isolated state fixture (no parser/DOM/Electron). The state fixture provides main/alt buffers, flags/stacks and a reply collector. This is **encoder/handler evidence, not native input or full parser execution**. Independent expected-string assertions included:

| Source invocation | Observed result |
|---|---|
| Handler set **63**, query | `CSI ?63u` (unknown bit 32 echoed) |
| Handler direct set **1**, push **3**, pop **1** | flags **0**, not restored direct-set base 1 |
| Encoder Ctrl+I, flags **1**, press | `CSI 105;5u` |
| Encoder Shift+A, flags **31**, press | `CSI 97:65;2;65u` |
| Encoder `a`, flags **31**, press **and repeat** | identical `CSI 97;;97u` |
| Encoder `a`, flags **31**, release | `CSI 97;1:3u` |
| Encoder Cyrillic `с`, physical `KeyC`, flags **31**, press | `CSI 1089;;1089u`, no base-layout key |

**Remaining automated acceptance, explicitly unexecuted:**

- **Independent pure contract/oracle tests:** parse derived output with an oracle not built from the same Alacritty serializer. Assert exact OSC8 `(ID,URI)` per cell through partial repaint, reflow, inactive-normal/active-alternate restore and close boundaries. Feed valid URI/ID at **8,192/256 bytes**, then **8,193/257**, invalid syntax and controls; require explicit failure, not truncation/automatic opening. Include inherited tenth-combining-character explicit failure in the overall model gate; it is not repaired by these keyboard findings.
- **Pure keyboard/query tests:** assert every set/OR/clear, main/alt push/pop/overflow/reset, MOK level/format and inter-protocol transition against the chosen profile. Include the Alacritty direct-set/query counterexample and VS Code text-repeat counterexample as fidelity failures unless a separately chosen supported implementation corrects them. Verify exactly one daemon reply and zero leaked host replies; byte-fragmentation, paste-looking-like-replies, delayed responses after takeover/detach, absent facts and cached detached facts.
- **Dedicated native-host input oracle:** in a separately authorized disposable terminal/profile, capture raw PTY bytes while an independently labeled event source supplies Escape, Tab/Ctrl+I, Enter/Ctrl+M, Ctrl+Shift variants, Alt/Option, arrows/F-keys/keypad, modifier-only keys, held-key repeat/release, Caps/NumLock, non-US layout, dead keys/IME and focus loss. Vary flags **0,1,3,7,11,15,31**, MOK **0/1/2**, settings enabled/disabled, main/alternate stack nesting and resets. Record genuine source-event labels versus emitted bytes, not a parser round-trip of hand-authored sequences. Native OS shortcuts/IME that cannot be injected faithfully require declared exclusions or a human check; do not invent events to make the oracle pass.
- **Dedicated native-host OSC8 oracle:** render arbitrary labels with absent/same/different IDs and same/different URIs, semicolons/percent-encoded Unicode, exact maximum lengths and restore/repaint. Observe host metadata/UI grouping via an authorized host-side oracle; screenshots alone cannot prove ID/target preservation, and raw PTY capture alone proves only bytes sent. Never automatically open/fetch targets. Terminal.app's unknowns require these checks before claiming support.

Research is evidence-complete **with explicit native uncertainties**, not host qualification or a declaration that all requirements are already implementable unchanged at the inspected pins.

## Primary sources

[a-manifest]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/Cargo.toml
[a-term]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/term/mod.rs
[a-key]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty/src/input/keyboard.rs
[a-ui]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty/src/config/ui_config.rs#L115-L129
[a-cell]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/term/cell.rs#L46-L106
[vte]: https://github.com/alacritty/vte/blob/3b3da71c34cc1256c7e20981cf03f8eb95e08ffc/src/ansi.rs
[apple]: https://support.apple.com/guide/terminal/change-profiles-keyboard-settings-trmlkbrd/2.15/mac/27
[i-tag]: https://github.com/gnachman/iTerm2/tree/v3.7.6
[i-term]: https://github.com/gnachman/iTerm2/blob/7ce456b98db38d3f0bafd70ffec87752e69d5d58/sources/VT100/VT100Terminal.m
[i-csi]: https://github.com/gnachman/iTerm2/blob/7ce456b98db38d3f0bafd70ffec87752e69d5d58/sources/VT100/VT100CSIParser.m#L590-L712
[i-modern]: https://github.com/gnachman/iTerm2/blob/7ce456b98db38d3f0bafd70ffec87752e69d5d58/sources/Keyboard/iTermModernKeyMapper.swift
[i-mok]: https://github.com/gnachman/iTerm2/blob/7ce456b98db38d3f0bafd70ffec87752e69d5d58/sources/Keyboard/iTermModifyOtherKeysMapper.m#L29-L199
[i-mok1]: https://github.com/gnachman/iTerm2/blob/7ce456b98db38d3f0bafd70ffec87752e69d5d58/sources/Keyboard/iTermModifyOtherKeysMapper1.m
[i-session]: https://github.com/gnachman/iTerm2/blob/7ce456b98db38d3f0bafd70ffec87752e69d5d58/sources/PTYSession/PTYSession.m
[i-profile]: https://github.com/gnachman/iTerm2/blob/7ce456b98db38d3f0bafd70ffec87752e69d5d58/sources/Settings/iTermProfilePreferences.m#L1251-L1257
[i-output]: https://github.com/gnachman/iTerm2/blob/7ce456b98db38d3f0bafd70ffec87752e69d5d58/sources/VT100/VT100Output.m#L1554-L1588
[i-advanced]: https://github.com/gnachman/iTerm2/blob/7ce456b98db38d3f0bafd70ffec87752e69d5d58/sources/Settings/iTermAdvancedSettingsModel.m#L456
[i-esc]: https://iterm2.com/documentation-escape-codes.html
[i-keys]: https://iterm2.com/documentation-preferences-profiles-keys.html
[vs-lock]: https://github.com/microsoft/vscode/blob/1.143.1/package-lock.json
[vs-config]: https://github.com/microsoft/vscode/blob/1.143.1/src/vs/workbench/contrib/terminal/common/terminalConfiguration.ts#L605-L611
[vs-xterm]: https://github.com/microsoft/vscode/blob/1.143.1/src/vs/workbench/contrib/terminal/browser/xterm/xtermTerminal.ts#L274-L283
[vs-instance]: https://github.com/microsoft/vscode/blob/1.143.1/src/vs/workbench/contrib/terminal/browser/terminalInstance.ts#L1157-L1218
[vs-links]: https://github.com/microsoft/vscode/blob/1.143.1/src/vs/workbench/contrib/terminalContrib/links/browser/terminalLinkManager.ts#L116-L148
[x-package]: https://registry.npmjs.org/@xterm/xterm/6.1.0-beta.304
[x-input]: https://github.com/xtermjs/xterm.js/blob/c58ea3637f3968e0e6e79cd92cf9aace7ef89ee2/src/common/InputHandler.ts
[x-key]: https://github.com/xtermjs/xterm.js/blob/c58ea3637f3968e0e6e79cd92cf9aace7ef89ee2/src/common/input/KittyKeyboard.ts
[x-service]: https://github.com/xtermjs/xterm.js/blob/c58ea3637f3968e0e6e79cd92cf9aace7ef89ee2/src/browser/services/KeyboardService.ts#L36-L62
[x-core]: https://github.com/xtermjs/xterm.js/blob/c58ea3637f3968e0e6e79cd92cf9aace7ef89ee2/src/browser/CoreBrowserTerminal.ts
[x-state]: https://github.com/xtermjs/xterm.js/blob/c58ea3637f3968e0e6e79cd92cf9aace7ef89ee2/src/common/services/CoreService.ts#L30-L71
[x-legacy]: https://github.com/xtermjs/xterm.js/blob/c58ea3637f3968e0e6e79cd92cf9aace7ef89ee2/src/common/input/Keyboard.ts
[x-links]: https://github.com/xtermjs/xterm.js/blob/c58ea3637f3968e0e6e79cd92cf9aace7ef89ee2/src/common/services/OscLinkService.ts#L30-L86
[kitty]: https://sw.kovidgoyal.net/kitty/keyboard-protocol/
[osc8]: https://gist.github.com/egmontkob/eb114294efbcd5adb1944c9f3cb5feda
[ctlseq]: https://invisible-island.net/xterm/ctlseqs/ctlseqs.html
[modified]: https://invisible-island.net/xterm/modified-keys.html
[xt-query]: https://github.com/ThomasDickey/xterm-snapshots/blob/xterm-411/charproc.c#L2468-L2507
