# xterm.js attack surface and mobile input limits

Research for [Establish xterm.js's attack surface and mobile input limits](https://github.com/jackgeek/climon2/issues/233), part of [Chart the web monitor specification](https://github.com/jackgeek/climon2/issues/229).

Sources are xterm.js's own documentation, source, issues and advisories, plus W3C, Chromium and WebKit documents. Source links are pinned to two revisions read on 2026-11-14:

- **6.0.0**, the latest release (tag commit `f447274`, 2025-12-22), linked as `6.0.0`.
- **master** at commit `f70764d` (2026-11-08), linked by full hash. It still declares version 6.0.0 in `package.json` but differs from the release in ways noted below.

Nothing here was run in a browser or on a phone. Claims about on-device behaviour that rest on issue reports are marked as such, and open items are collected under [Unverified](#unverified).

## Summary

1. **xterm.js core never puts session text into the page as markup.** The DOM renderer and the accessibility tree write cells with `textContent`; no `innerHTML`, `eval` or script loading exists in core. The page-level danger is in what the embedder does with the values xterm.js hands back (title, link URI, buffer text) and in which addons it loads.
2. **Beyond drawing cells, core acts on a short list of sequences:** OSC 0/2 (title event), OSC 8 (hyperlinks), OSC 4/10/11/12/104/110-112 (palette set and query), DECRQSS, device and mode queries, a gated subset of window operations, and mode switches for mouse, focus and bracketed paste. OSC 52, images (SIXEL, iTerm, Kitty), OSC 9 progress and OSC 7501 exist only in addons. The web view should load none of those addons.
3. **Query replies and user keystrokes leave xterm.js through the same `onData` event**, with no public flag to tell them apart. The session daemon is the sole query authority (ADR 0007), so the web view must make sure no query reaches xterm.js, or swallow them with parser hooks.
4. **The session daemon's filter and xterm.js disagree about C1 controls.** xterm.js treats the code points U+0080 to U+009F as sequence introducers (U+009B CSI, U+009D OSC, U+0090 DCS, U+009F APC). The daemon's tokenizer only starts a sequence at `ESC`, classifies every other byte as text and forwards text unchanged. Sequences the daemon believes it strips (OSC 8, palette and device queries, DECRQSS, window operations) can therefore still reach xterm.js in C1 form. The web monitor or web view must neutralise C1 code points before `write`, and the web view must be safe even if that filter fails. This is the main finding.
5. **OSC 8 links are the one click-through path.** With no `linkHandler`, activation shows a browser `confirm` and then opens the URI in a new window; only `http:` and `https:` are offered unless the embedder opts in. The web view should set its own `linkHandler` that either does nothing or shows the full URI in web view chrome before opening.
6. **Advisories:** one, CVE-2019-0542 (remote code execution, fixed in 3.8.1, 3.9.2 and 3.10.1). The repository has no `SECURITY.md` and lists no advisories of its own.
7. **Strict CSP:** scripts need no relaxation in core (no eval, no workers, no WebAssembly). Styles do: core creates `<style>` elements and the DOM renderer sets `style` attributes, there is no nonce option, and the upstream issue is unresolved. A strict `style-src` therefore needs either the WebGL renderer plus tolerated breakage, `'unsafe-inline'` for styles only, or a patched build. This needs a prototype before the asset-delivery ticket decides.
8. **Mobile is unsupported territory upstream.** Maintainers describe mobile as a blind spot and IME handling as out of scope, and recommend a custom keyboard or key bar. Release 6.0.0 wires no touch handling at all; master adds touch scrolling only. Touch selection and copy are an open issue. Android soft-keyboard input is an open issue with reports through 6.0.0. xterm.js never resizes itself, and on iOS and current Android Chrome the on-screen keyboard changes only the visual viewport, so the web view must drive sizing from `visualViewport`.

## Untrusted bytes and the page

### How cells reach the DOM

- The DOM renderer sets each span's content with `textContent` ([`DomRendererRowFactory.ts`](https://github.com/xtermjs/xterm.js/blob/f70764dd3da05ebda0390605c6f9adbbea3f0e90/src/browser/renderer/dom/DomRendererRowFactory.ts#L231)), as does the accessibility tree ([`AccessibilityManager.ts`](https://github.com/xtermjs/xterm.js/blob/f70764dd3da05ebda0390605c6f9adbbea3f0e90/src/browser/AccessibilityManager.ts#L190)) and the IME composition view ([`CompositionHelper.ts`](https://github.com/xtermjs/xterm.js/blob/f70764dd3da05ebda0390605c6f9adbbea3f0e90/src/browser/input/CompositionHelper.ts#L92)).
- A search of `src/` and `addons/*/src` on master for `innerHTML`, `insertAdjacentHTML`, `outerHTML`, `document.write`, `eval(`, `new Function`, `DOMParser`, `new Worker`, `fetch(` and `XMLHttpRequest` finds none outside tests. On 6.0.0 the only hit is a comment in vendored VS Code code.
- The maintainers make the same point in the issue that led to the security guide: the DOM renderer is unaffected because it inserts with `textContent`, and the risk sits in API surfaces such as the title event and the linkifier ([#2443](https://github.com/xtermjs/xterm.js/issues/2443)).
- The WebGL addon draws to a canvas ([`WebglRenderer.ts`](https://github.com/xtermjs/xterm.js/blob/f70764dd3da05ebda0390605c6f9adbbea3f0e90/addons/addon-webgl/src/WebglRenderer.ts#L91)).

### What core does with sequences, beyond drawing

All handler registrations are in [`InputHandler.ts`](https://github.com/xtermjs/xterm.js/blob/f70764dd3da05ebda0390605c6f9adbbea3f0e90/src/common/InputHandler.ts#L205-L375); the same OSC and DCS set is present in [6.0.0](https://github.com/xtermjs/xterm.js/blob/6.0.0/src/common/InputHandler.ts#L292-L381).

| Sequence | Effect in core | What reaches the page or the session |
| --- | --- | --- |
| OSC 0, OSC 2 | Stores the string and fires `onTitleChange` with it unmodified ([`setTitle`](https://github.com/xtermjs/xterm.js/blob/f70764dd3da05ebda0390605c6f9adbbea3f0e90/src/common/InputHandler.ts#L3043-L3047)). xterm.js does not touch `document.title`. | Only what the embedder does with the string. |
| OSC 1 | Stored, never exposed. | Nothing. |
| OSC 8 | Attaches a URI to cells ([`setHyperlink`](https://github.com/xtermjs/xterm.js/blob/f70764dd3da05ebda0390605c6f9adbbea3f0e90/src/common/InputHandler.ts#L3110)). | A clickable link; see below. |
| OSC 4, 10, 11, 12, 104, 110-112 | Sets, restores or reports palette, foreground, background and cursor colours ([`CoreBrowserTerminal.ts`](https://github.com/xtermjs/xterm.js/blob/f70764dd3da05ebda0390605c6f9adbbea3f0e90/src/browser/CoreBrowserTerminal.ts#L225-L250)). | Session output can recolour the terminal; a query produces a reply on `onData`. |
| DCS `$ q` (DECRQSS) | Replies with fixed, internally generated strings ([`requestStatusString`](https://github.com/xtermjs/xterm.js/blob/f70764dd3da05ebda0390605c6f9adbbea3f0e90/src/common/InputHandler.ts#L3517-L3535)). | A reply on `onData`. |
| CSI `c`, `> c`, `n`, `? n`, `$ p`, `> q` | Device attributes, status, cursor position, mode and version reports. | Replies on `onData`. |
| CSI `t` (window operations) | Every operation is refused unless its flag in `windowOptions` is set; the check also wraps custom handlers ([gate](https://github.com/xtermjs/xterm.js/blob/f70764dd3da05ebda0390605c6f9adbbea3f0e90/src/common/InputHandler.ts#L53-L82), [custom-handler guard](https://github.com/xtermjs/xterm.js/blob/f70764dd3da05ebda0390605c6f9adbbea3f0e90/src/common/InputHandler.ts#L678-L688)). | Nothing by default. |
| Mouse modes, focus reporting (`? 1004`), bracketed paste (`? 2004`) | Change what user gestures, focus changes and pastes send. | Mouse, focus and paste-marker bytes on `onData`. |
| Kitty keyboard (`CSI = ? > < u`), win32 input mode (`? 9001`) | Master only, and only when the embedder enables `vtExtensions.kittyKeyboard` or `vtExtensions.win32InputMode` ([typings](https://github.com/xtermjs/xterm.js/blob/f70764dd3da05ebda0390605c6f9adbbea3f0e90/typings/xterm.d.ts#L467-L492)). Not in 6.0.0. | Changes key encoding. |

Core registers no handler for OSC 52, OSC 7, OSC 9, OSC 1337, SIXEL or APC. The documentation states that all window manipulation and report features are off by default for security reasons ([`ITerminalOptions.windowOptions`](https://xtermjs.org/docs/api/terminal/interfaces/iterminaloptions/)), and [`IWindowOptions`](https://xtermjs.org/docs/api/terminal/interfaces/iwindowoptions/) explains the reason: many of them reveal facts about the machine running the terminal. Only size reports (14, 16, 18) and the title stack (22, 23) have built-in implementations; the title stack is capped at 10 entries ([source](https://github.com/xtermjs/xterm.js/blob/f70764dd3da05ebda0390605c6f9adbbea3f0e90/src/common/InputHandler.ts#L2955-L2980)).

**Defence.** Leave `windowOptions` empty. Do not enable `vtExtensions`. Treat the `onTitleChange` string as text only; the web monitor's title should come from the daemon's structured title metadata, not from this event.

### Replies share the input channel

Every reply above is sent with `triggerDataEvent`, the same call that carries keystrokes, and both fire the single public `onData` event ([`CoreService.ts`](https://github.com/xtermjs/xterm.js/blob/f70764dd3da05ebda0390605c6f9adbbea3f0e90/src/common/services/CoreService.ts#L74-L95)). The internal `wasUserInput` flag is not part of what `onData` delivers. `disableStdin` suppresses both together.

ADR 0007 makes the session daemon the sole authority for terminal-query replies, and the terminal-integration contract says a monitor's parser must not send duplicate replies. A web view that forwards `onData` to the session would break that whenever a query reaches xterm.js.

**Defence.** Keep queries from reaching xterm.js (the daemon filter plus the C1 defence below), and add parser hooks that swallow them as a second layer. Custom handlers are probed before built-in ones and returning `true` stops processing ([parser hooks guide](https://xtermjs.org/docs/guides/hooks/)). The same guide warns that hooks cannot intercept C0 or C1 controls or printed text, and that OSC and DCS payloads handed to a hook are untrusted.

### C1 controls bypass the daemon's filter

- xterm.js decodes input to code points and then parses them ([`InputHandler.ts`](https://github.com/xtermjs/xterm.js/blob/f70764dd3da05ebda0390605c6f9adbbea3f0e90/src/common/InputHandler.ts#L476-L493)). Its transition table treats U+009B as CSI, U+009D as OSC, U+0090 as DCS, U+009F as APC, U+0098 and U+009E as SOS and PM, and U+009C as the string terminator, from any state ([`EscapeSequenceParser.ts`](https://github.com/xtermjs/xterm.js/blob/f70764dd3da05ebda0390605c6f9adbbea3f0e90/src/common/parser/EscapeSequenceParser.ts#L121-L130)).
- The session daemon's tokenizer begins a sequence only at the byte `ESC`; in its ground state every other byte, 0x00 to 0xFF, is text ([`token.rs`](../../crates/climon2-vt/src/token.rs)), and the live output policy forwards text tokens unchanged ([`classify.rs`](../../crates/climon2-vt/src/classify.rs)). A C1 code point arrives as an ordinary two-byte UTF-8 character and is text to the daemon.

So the daemon's guarantee that OSC, DCS, APC and host-dependent CSI requests never reach the consumer does not hold for xterm.js as the consumer. This applies to Direct presentation, where filtered tokens are forwarded. What `climon2-vt`'s engine does with these code points, and so whether restoration or Derived presentation can re-emit them, was not established.

**Defence.** The web monitor (preferred, so every web view benefits) or the web view must replace or drop U+0080 to U+009F in session bytes before they reach `Terminal.write`. This belongs in the threat-model and browser-protocol tickets, and it suggests asking whether the daemon filter should handle C1 itself; that would touch ADR 0007 and must be raised explicitly, not decided here. Independently, the web view must not depend on the filter: no risky addons, an inert or confirming `linkHandler`, empty `windowOptions`, and query-swallowing hooks.

### OSC 8 hyperlinks

- Links whose URI does not parse, or whose scheme is not `http:` or `https:`, are ignored unless `linkHandler.allowNonHttpProtocols` is set ([`OscLinkProvider.ts`](https://github.com/xtermjs/xterm.js/blob/f70764dd3da05ebda0390605c6f9adbbea3f0e90/src/browser/OscLinkProvider.ts#L64-L75)). The 5.1.0 release notes describe this as forcing the embedder to opt in to other schemes ([releases](https://github.com/xtermjs/xterm.js/releases)).
- With no `linkHandler`, activation calls `confirm` with the URI and a warning, then opens a new window, clears `opener` and navigates it (same file, `defaultActivate`). The option's documentation confirms the `confirm` default and tells embedders to think about the safety of users opening links ([`linkHandler`](https://xtermjs.org/docs/api/terminal/interfaces/iterminaloptions/)).
- The link text shown in cells is independent of the URI. The link-handling guide recommends a modifier key and a hover popup showing the real target for that reason ([guide](https://xtermjs.org/docs/guides/link-handling/)). A phone has neither hover nor a modifier key.
- The web-links addon detects `http(s)` URLs in plain text and opens them in a new window with `opener` cleared and **no confirmation** ([`WebLinksAddon.ts`](https://github.com/xtermjs/xterm.js/blob/f70764dd3da05ebda0390605c6f9adbbea3f0e90/addons/addon-web-links/src/WebLinksAddon.ts#L21-L36)).

The daemon strips OSC 8 in `ESC ]` form, so links should only appear through the C1 gap. **Defence.** Always set `linkHandler`, with `allowNonHttpProtocols` unset. Either make `activate` a no-op, or have it show the complete URI in web view chrome and require an explicit tap to open with `noopener` and `noreferrer`. Do not load the web-links addon without the same handler. A tap-to-open link is a route by which hostile session output can send the credential-bearing browser to an attacker's page, so the threat-model ticket should decide whether links exist at all.

### Addons

| Addon | Registers | Notes |
| --- | --- | --- |
| clipboard | OSC 52 ([`ClipboardAddon.ts`](https://github.com/xtermjs/xterm.js/blob/f70764dd3da05ebda0390605c6f9adbbea3f0e90/addons/addon-clipboard/src/ClipboardAddon.ts#L20)) | Default provider calls `navigator.clipboard.readText` and `writeText`: session output could read or replace the device clipboard. Do not load. |
| image | SIXEL (DCS `q`), iTerm OSC 1337, Kitty APC `G`, plus CSI hooks ([`ImageAddon.ts`](https://github.com/xtermjs/xterm.js/blob/f70764dd3da05ebda0390605c6f9adbbea3f0e90/addons/addon-image/src/ImageAddon.ts#L152-L205)) | Decodes attacker-supplied images, partly in WebAssembly and partly by the browser; turns on three `windowOptions` size reports by default; defaults allow 32 MB per sequence and 128 MB of storage ([README](https://github.com/xtermjs/xterm.js/blob/f70764dd3da05ebda0390605c6f9adbbea3f0e90/addons/addon-image/README.md)). Do not load. |
| progress | OSC 9 | climon2 handles progress in the daemon. Do not load. |
| program-status | OSC 7501 (master only) | climon2 handles this in the daemon. Do not load. |
| web-links | Link provider over buffer text | See above. |
| attach | WebSocket plumbing | The security guide says neither it nor the demo is a basis for production use because they carry no security measures. Do not use. |
| serialize | None | 6.0.0 fixed missing HTML escaping in its HTML output ([releases](https://github.com/xtermjs/xterm.js/releases)). Not needed. |
| fit, webgl, unicode | None | Candidates. |

The security guide notes that core has no third-party runtime dependencies and that addons follow different rules ([guide](https://xtermjs.org/docs/guides/security/)). The image addon is the example: it imports WebAssembly decoders from a separate package ([`IIPHandler.ts`](https://github.com/xtermjs/xterm.js/blob/f70764dd3da05ebda0390605c6f9adbbea3f0e90/addons/addon-image/src/IIPHandler.ts#L9-L10)).

### Paste

Paste text goes to the session through `onData`. On master, when bracketed paste is active, `ESC` inside pasted text is replaced by a visible symbol so the text cannot close the bracket early ([`Clipboard.ts`](https://github.com/xtermjs/xterm.js/blob/f70764dd3da05ebda0390605c6f9adbbea3f0e90/src/browser/Clipboard.ts#L21-L29)). Release 6.0.0 wraps the text without that step ([`Clipboard.ts` at 6.0.0](https://github.com/xtermjs/xterm.js/blob/6.0.0/src/browser/Clipboard.ts#L16-L25)). The terminal-integration contract requires the paste payload to be preserved exactly and markers sent only when the application enabled the mode, so the web view should probably own paste itself instead of relying on either behaviour.

### Resource limits

- OSC, DCS and APC payloads are capped at 10 MB each ([`Constants.ts`](https://github.com/xtermjs/xterm.js/blob/f70764dd3da05ebda0390605c6f9adbbea3f0e90/src/common/parser/Constants.ts#L65-L67), [hooks guide](https://xtermjs.org/docs/guides/hooks/)). The daemon already drops any control string over 4096 bytes in `ESC` form.
- The write buffer discards data beyond roughly 50 MB of pending input; master throws an error at that point ([`WriteBuffer.ts`](https://github.com/xtermjs/xterm.js/blob/f70764dd3da05ebda0390605c6f9adbbea3f0e90/src/common/input/WriteBuffer.ts#L155-L157)). The flow-control guide gives a processing rate of 5 to 35 MB/s, recommends a high watermark under 500 KB driven by `write` callbacks, and says a WebSocket cannot provide back pressure by itself, so the client should acknowledge processed chunks ([guide](https://xtermjs.org/docs/guides/flowcontrol/)). A phone will be slower. The browser protocol needs acknowledgement-based flow control, which also bounds memory against a flooding session.
- `scrollback` defaults to 1000 rows ([options](https://xtermjs.org/docs/api/terminal/interfaces/iterminaloptions/)).

### Security guidance and advisories

The [security guide](https://xtermjs.org/docs/guides/security/) is general. Points that bear on the web view:

- Any script on the page has the terminal's shell access and sees every keystroke, so avoid third-party resources, runtime-fetched script and iframes, and prebundle assets so they can be audited.
- Treat all terminal output as untrusted, naming `onTitleChange`, the linkifier interfaces, `Terminal.buffer` access and custom parser hooks, and open terminal-supplied links in a separate context.
- For a large application, give the terminal its own scripting context.
- Use `wss` and add authentication and authorisation on top of the messages.

Advisories:

- [CVE-2019-0542 / GHSA-mc23-976p-j42x](https://github.com/advisories/GHSA-mc23-976p-j42x): remote code execution from mishandled special characters, CVSS 8.8, affecting versions before 3.8.1, 3.9.0 to 3.9.1 and 3.10.0; fixed in 3.8.1, 3.9.2 and 3.10.1 by [this commit](https://github.com/xtermjs/xterm.js/commit/3592c641cd0348e0b698e8a180aea2072c6bcd9a). No later advisory for xterm.js was found.
- The repository's [security page](https://github.com/xtermjs/xterm.js/security) shows no security policy and no published advisories, so there is no documented private reporting channel or supported-versions statement.
- 5.4.0 moved packages to the `@xterm` npm scope, stated as mainly a security measure against name confusion ([releases](https://github.com/xtermjs/xterm.js/releases)). The web view must depend on `@xterm/xterm`, not the deprecated `xterm`.

## Content security policy

No xterm.js document describes CSP. What follows is from source and two upstream issues.

**Script.** Core contains no `eval`, `new Function`, worker creation, WebAssembly or network fetch (search above), so `script-src 'self'` should suffice for core, the fit addon and the WebGL addon. The image addon would need WebAssembly compilation, which CSP gates behind `'wasm-unsafe-eval'` ([CSP3](https://www.w3.org/TR/CSP3/), 4.5.1); another reason not to load it.

**Style.** Three things conflict with a `style-src` that lacks `'unsafe-inline'`:

1. Core's viewport creates a `<style>` element for scrollbar colours whatever renderer is used ([`Viewport.ts`](https://github.com/xtermjs/xterm.js/blob/f70764dd3da05ebda0390605c6f9adbbea3f0e90/src/browser/Viewport.ts#L83-L98)).
2. The DOM renderer creates two `<style>` elements, for dimensions and theme ([`DomRenderer.ts`](https://github.com/xtermjs/xterm.js/blob/f70764dd3da05ebda0390605c6f9adbbea3f0e90/src/browser/renderer/dom/DomRenderer.ts#L162-L185)).
3. The DOM renderer sets per-cell colours with `setAttribute('style', ...)` ([`DomRendererRowFactory.ts`](https://github.com/xtermjs/xterm.js/blob/f70764dd3da05ebda0390605c6f9adbbea3f0e90/src/browser/renderer/dom/DomRendererRowFactory.ts#L530-L532)).

CSP blocks inline style blocks unless the policy allows them by `'unsafe-inline'`, nonce or hash, and governs style attributes through `style-src-attr` ([CSP3](https://www.w3.org/TR/CSP3/), 6.1.13 to 6.1.15). xterm.js has no nonce option: the string does not occur in its source or typings. The same three sites exist in 6.0.0.

Upstream state:

- [#4133](https://github.com/xtermjs/xterm.js/issues/4133) (5.0.0): the DOM renderer always initialises at `open()` before a renderer addon takes over, so its inline styles trigger violations even when WebGL is used; the reporter saw only console noise once the other renderer loaded.
- [#4445](https://github.com/xtermjs/xterm.js/issues/4445) (5.1.0 onwards): under `style-src 'self'` the blocked styles broke resizing. A maintainer was open to a nonce option or CSS variables but said the fix should come from someone affected, since VS Code is not. An attempt with constructed stylesheets was reverted after a Chromium crash; a later pull request moving colours to CSS custom properties was closed unmerged. No fix has landed.

The values written into these styles are generated colour strings and pixel sizes, not session text: per-cell colours are formatted from numbers ([source](https://github.com/xtermjs/xterm.js/blob/f70764dd3da05ebda0390605c6f9adbbea3f0e90/src/browser/renderer/dom/DomRendererRowFactory.ts#L414)), and palette changes from OSC 4/10/11/12 pass through numeric channels ([source](https://github.com/xtermjs/xterm.js/blob/f70764dd3da05ebda0390605c6f9adbbea3f0e90/src/browser/CoreBrowserTerminal.ts#L241-L246)). `fontFamily` is interpolated into the theme stylesheet unescaped, so it must stay a constant chosen by climon2.

**Images.** `xterm.css` uses a `data:` SVG as a mask image ([`xterm.css`](https://github.com/xtermjs/xterm.js/blob/f70764dd3da05ebda0390605c6f9adbbea3f0e90/css/xterm.css#L230-L231)); `img-src` governs image loads ([CSP3](https://www.w3.org/TR/CSP3/), 6.1.6), so a policy without `data:` there will probably lose that glyph.

**Options for the asset-delivery ticket**, none yet tested:

- `script-src 'self'` with `style-src 'self' 'unsafe-inline'`. Simple; gives up CSP as a barrier against style injection but keeps it against script. Since xterm.js writes cells with `textContent`, there is no known route for session bytes to become markup that could carry a style.
- Strict styles with the WebGL renderer, accepting whatever the blocked viewport and DOM-renderer styles break. #4445 says this is more than cosmetic in some versions.
- A patched or forked build that moves the three sites to the stylesheet and CSS custom properties. Carries a maintenance cost and defeats "unmodified upstream".

Whichever is chosen, the policy should also set `default-src 'none'`, `connect-src 'self'`, `base-uri 'none'`, `form-action 'none'` and `frame-ancestors 'none'`; those are general hardening and not xterm.js-specific.

## Mobile input and viewport

### Upstream position

- In [#5377](https://github.com/xtermjs/xterm.js/issues/5377) a maintainer says mobile support hardly exists beyond what browsers map from touch to mouse, that the front end and CI are desktop-only, and that the key-handling code is old and fragile. The other maintainer says IME input handling is out of scope, suggests a custom keyboard or key bar as the workaround, and says touch is not a priority for him.
- [#1101](https://github.com/xtermjs/xterm.js/issues/1101) is the long-standing umbrella issue for mobile.

### Soft keyboard, IME and composition

- All keyboard input goes through a hidden `<textarea>` with `autocomplete`, `autocorrect`, `autocapitalize` and `spellcheck` turned off ([`CoreBrowserTerminal.ts`](https://github.com/xtermjs/xterm.js/blob/f70764dd3da05ebda0390605c6f9adbbea3f0e90/src/browser/CoreBrowserTerminal.ts#L487-L501)). In [#2403](https://github.com/xtermjs/xterm.js/issues/2403) the reporter found that `autocomplete="off"` did not stop Android's predictive text; the issue was closed into #1101 without a fix.
- Composition is handled by reading the textarea after `compositionend`, because the event's own data is unreliable, and by diffing the textarea when a key event with code 229 arrives outside a composition; a shrink is sent as a single DEL ([`CompositionHelper.ts`](https://github.com/xtermjs/xterm.js/blob/f70764dd3da05ebda0390605c6f9adbbea3f0e90/src/browser/input/CompositionHelper.ts#L113-L235)). The UI Events specification assigns key code 229 to any keydown an IME is processing ([UI Events](https://w3c.github.io/uievents/), 7.3.1), says key events during composition may be suppressed or have values withheld by the IME (4.3.3), and says keyboard events cannot reveal IME state. Android soft keyboards compose nearly everything, so on Android xterm.js is mostly running this fallback path.
- A second path forwards `input` events of type `insertText` when no keydown was seen ([`CoreBrowserTerminal.ts`](https://github.com/xtermjs/xterm.js/blob/f70764dd3da05ebda0390605c6f9adbbea3f0e90/src/browser/CoreBrowserTerminal.ts#L1039-L1058)).
- [#3600](https://github.com/xtermjs/xterm.js/issues/3600), open, reports jumbled, duplicated or lost characters and misbehaving Backspace and Enter on Android Chrome with Gboard, from 4.14.1 through 6.0.0. A maintainer called it very low priority. One sub-case (single-character backspace) was fixed.
- 6.0.0 fixed duplicated input for some IMEs ([releases](https://github.com/xtermjs/xterm.js/releases)).

**Implication.** The web view should not rely on xterm.js's textarea for phone typing. The safer design is the one the maintainer suggests: a web-view-owned input element (HTML and TypeScript, as the map already requires for chrome) whose committed text the web view sends itself, with xterm.js used for display. `disableStdin` makes the helper textarea read-only and silences `onData`.

### Modifier and special keys

Phone keyboards have no Ctrl, Alt, Esc, Tab, arrows or function keys. xterm.js offers no on-screen keys. It offers `attachCustomKeyEventHandler` and `Terminal.input(data, wasUserInput)`, which injects bytes as if typed ([typings](https://github.com/xtermjs/xterm.js/blob/f70764dd3da05ebda0390605c6f9adbbea3f0e90/typings/xterm.d.ts#L1145-L1202)). 6.0.0 removed the built-in Alt-to-Ctrl+arrow mapping and expects embedders to add their own bindings ([releases](https://github.com/xtermjs/xterm.js/releases)). The web view's mobile controls must therefore supply a key bar, including the client escape prefix and take-control key, and must encode arrows according to the application cursor mode it learns from the session.

### Touch scrolling

- **6.0.0:** nothing under `src/browser` listens for touch events. A vendored gesture class exists but is only referenced to ignore gestures ([`widget.ts` at 6.0.0](https://github.com/xtermjs/xterm.js/blob/6.0.0/src/vs/base/browser/ui/widget.ts#L54-L56)). A commenter on #5377 says scrolling got worse on phones in 6.0.0.
- **master:** a gesture recogniser listens on the document with non-passive `touchstart` and `touchmove` ([`touch.ts`](https://github.com/xtermjs/xterm.js/blob/f70764dd3da05ebda0390605c6f9adbbea3f0e90/src/browser/scrollable/touch.ts#L215-L217)), and a vertical drag is routed three ways ([`MouseService.ts`](https://github.com/xtermjs/xterm.js/blob/f70764dd3da05ebda0390605c6f9adbbea3f0e90/src/browser/services/MouseService.ts#L296-L318)): as mouse-wheel reports when the application enabled wheel tracking, as up and down arrow keys on the alternate screen, and otherwise as scrollback scrolling. The first two send bytes to the session.

No release contains the master behaviour yet. Pinning 6.0.0 means the web view implements touch scrolling itself; tracking a pre-release means accepting unreleased code in a security-sensitive page.

### Touch selection and copy

Selection is driven by mouse events only ([`SelectionService.ts`](https://github.com/xtermjs/xterm.js/blob/f70764dd3da05ebda0390605c6f9adbbea3f0e90/src/browser/services/SelectionService.ts#L508-L509)), and `.xterm` sets `user-select: none` ([`xterm.css`](https://github.com/xtermjs/xterm.js/blob/f70764dd3da05ebda0390605c6f9adbbea3f0e90/css/xterm.css#L39-L45)), so the browser's native long-press selection is off too. [#3727](https://github.com/xtermjs/xterm.js/issues/3727), open, reports that text cannot be selected or copied by touch on iPhone, iPad or Android with any renderer. A maintainer thinks the DOM renderer or screen-reader mode might allow it on iPad; a later commenter says the DOM renderer did not help on iPhone with 5.3.0. The accessibility tree, present only with `screenReaderMode`, is the one part of the terminal with `user-select: text`.

**Implication.** Copy on a phone needs a web-view-owned feature, for example a "copy mode" overlay built from `Terminal.buffer` text inserted with `textContent`, as one commenter on #3727 did. The agreed interactive monitor has no clipboard engine either, so parity does not require this; it is a web view decision.

### Viewport and the on-screen keyboard

- xterm.js does not watch its container. The fit addon computes rows and columns from the parent element's computed height and width only when `fit()` is called ([`FitAddon.ts`](https://github.com/xtermjs/xterm.js/blob/f70764dd3da05ebda0390605c6f9adbbea3f0e90/addons/addon-fit/src/FitAddon.ts#L56-L80)).
- Since Chrome 108 on Android, showing the on-screen keyboard resizes only the visual viewport; the layout viewport and viewport units stay unchanged. iOS Safari behaves the same way, and fixed-position elements can end up under the keyboard. Chrome accepts `interactive-widget=resizes-content` in the viewport meta tag to restore layout resizing; that key is not supported on iOS ([Chrome for Developers](https://developer.chrome.com/blog/viewport-resize-behavior)).
- `window.visualViewport` exposes `width`, `height`, `offsetTop`, `offsetLeft` and `scale`, with `resize` and `scroll` events ([Chrome for Developers](https://developer.chrome.com/blog/visual-viewport-api)). WebKit shipped it in Safari 13 and describes it as reporting the visible part of the page with zoom and the on-screen keyboard taken into account ([WebKit blog](https://webkit.org/blog/9674/new-webkit-features-in-safari-13/)).
- The VirtualKeyboard API, which reports keyboard geometry directly, is Chromium-only; WebKit's implementation request has been open since 2021 ([WebKit bug 230225](https://bugs.webkit.org/show_bug.cgi?id=230225)).

**Implication.** A container sized by `100vh` or `100%` will not shrink when the keyboard opens, `fit()` will see no change, and the cursor row can sit under the keyboard. The web view must size the terminal container from `visualViewport.height` and call `fit()` on `visualViewport` `resize`. Each fit changes the controller's size and therefore session geometry, so the web view behaviour ticket must decide whether opening the keyboard resizes the session (and causes every full-screen application to redraw) or keeps geometry and scrolls the cursor into view.

## What the session daemon already filters

From [ADR 0007](../adr/0007-daemon-owned-virtual-terminal.md), [terminal integration](../protocol/terminal-integration.md) and the implementing source in [`crates/climon2-vt`](../../crates/climon2-vt/src):

- **Stripped before any controller sees them:** every OSC (including OSC 8 and OSC 52), every DCS, SOS, PM and APC, keyboard-protocol negotiation, and every host-dependent CSI request. The CSI list in [`classify.rs`](../../crates/climon2-vt/src/classify.rs) covers final bytes `c`, `n`, `x` and `t` with any prefix, plus `> q`, `? S`, `$ p`, the four `u` keyboard-protocol forms, and `> m` and `? m`. C0 controls executed inside a stripped token are still forwarded.
- **Answered by the daemon and stripped:** DA1, DA2, DECID, DSR 5 and 6, DECRQM and `CSI 18 t`.
- **Titles:** tracked by value and re-emitted as `OSC 2` with C0, DEL and C1 controls percent-encoded ([`title.rs`](../../crates/climon2-vt/src/title.rs)). The agreed daemon wire also carries title and notification text as inert Unicode JSON metadata ([core-v1](../protocol/daemon-client/core-v1.md)), and the terminal-integration contract calls session titles inert labels.
- **Size limits:** a CSI or control-string body over 4096 bytes is dropped whole.
- **Modes 47 and 1047** are removed from mode sequences.
- **Forwarded unchanged:** text, C0 controls, plain escape sequences, and all other CSI sequences. That includes SGR, cursor movement, scroll regions, alternate screen 1049, mouse-tracking modes, focus reporting, bracketed paste and synchronized output.
- **Restoration** never replays raw history, so historical side effects do not repeat.
- **Planned, not implemented:** OSC 7501, OSC 133 and OSC 9/99/777 are observed by the daemon and delivered as structured metadata; the raw sequences stay stripped.

Measured against xterm.js's surface:

| xterm.js behaviour | Covered by the daemon in `ESC` form | Residual for the web monitor |
| --- | --- | --- |
| OSC 8 links | Yes | C1 form; set an inert or confirming `linkHandler`. |
| OSC 52 clipboard | Yes | C1 form, and only if the addon is loaded; do not load it. |
| OSC 0/2 title event | Raw titles stripped; a sanitised OSC 2 is emitted | The web view still receives `onTitleChange`; render as text or ignore. |
| Palette OSC set and query | Yes | C1 form: recolouring and replies. |
| DECRQSS and device queries | Yes | C1 form: replies on `onData`. |
| Window operations | `CSI t` stripped | Also off by default in xterm.js; keep `windowOptions` empty. |
| Images, progress, program status | Yes | Addons only; do not load them. |
| Mouse, focus and bracketed-paste modes | No, forwarded by design | Legitimate; touch gestures and focus changes will send bytes. |
| Output volume | Token size only | Flow control and write-buffer limits are the web monitor's job. |

## Unverified

- Whether `climon2-vt`'s engine (Alacritty 0.26.0, vte 0.15.0) treats U+0080 to U+009F as controls, prints them, or drops them, and so whether restoration or Derived presentation can emit them. The vte source was not available to read. The Direct-presentation gap is established from climon2's and xterm.js's own source, but no end-to-end test was run.
- Whether the monitor-mode byte stream the web monitor will forward contains the re-emitted `OSC 2` title, or only structured title metadata.
- Every statement about on-device behaviour (Android and Gboard input, iOS selection, 6.0.0 touch scrolling). These come from xterm.js issue reports and source reading, not from testing on a phone.
- Open or closed state of xterm.js issues #4445, #4133, #5377 and #1101: the fetched pages did not show it. #3727 was shown as open; #2403 as closed; #3600 appeared open.
- Whether a `style-src` without `'unsafe-inline'` leaves 6.0.0 or master usable with the WebGL renderer, and what exactly breaks. #4445's reports predate the 6.0.0 scrollbar rewrite.
- That assigning `element.style.<property>` from script is not subject to `style-src`. This is the premise of the CSS-variable fix proposed in #4445; the CSP specification text read here does not state it, and the specification's note on CSSOM is marked as needing clarification.
- That a `data:` mask image in `xterm.css` is blocked by an `img-src` without `data:`, and what is visibly lost.
- The exact change that fixed CVE-2019-0542. The advisory text does not name the sequence, and the linked fix commit was not read.
- That no xterm.js advisory exists after 2019. Only GitHub's advisory entry and the repository's security page were checked.
- The contents of the 4.x release notes and of the 5.0.0 breaking-change list, which were cut off when fetched.
- UI Events section numbers are from the editor's draft as fetched on 2026-11-14 and may shift.
- Firefox for Android and Samsung Internet behaviour for the on-screen keyboard and viewport; only Chrome and Safari sources were read.
