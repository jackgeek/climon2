# What browsers enforce for a private-origin web view

Research for [Establish what browsers enforce for a private-origin terminal page](https://github.com/jackgeek/climon2/issues/230), part of [Chart the web monitor specification](https://github.com/jackgeek/climon2/issues/229).

Researched 2026-11-14 against specifications, MDN, and browser vendor documentation, source and bug trackers. "Chrome" means Chrome on desktop and Android; Chrome on iOS is discussed with Safari. Statements marked *inference* are my reading of how cited rules combine, not something a source states outright. Anything I could not confirm is in [Unverified](#unverified).

Terms: the *web monitor* is the program `c2 web` runs; the *web view* is its browser side. "Private origin" means the web view was loaded from a loopback address, a LAN address, a tailnet address or name, or a tunnel hostname.

## Summary

1. **Only HTTPS and loopback are secure contexts.** `http://127.0.0.1`, `http://[::1]` and `http://localhost` qualify; `http://<LAN IP>`, `http://<tailnet IP>` and plain-HTTP names do not. Without a secure context the web view has no `SubtleCrypto`, no WebAuthn, no service worker, no async clipboard, no notifications and no wake lock. A phone therefore needs HTTPS for anything beyond a bare page and a WebSocket.
2. **Safari is the outlier on loopback.** It reports `http://localhost` as a secure context but, as of April 2026, still refuses `Secure` cookies there and still treats loopback as mixed content in places. Do not design the loopback case around `Secure` or `__Host-` cookies.
3. **The browser does not stop cross-site WebSocket hijacking; the web monitor must.** A WebSocket handshake is sent cross-origin with cookies and without a preflight. The only handshake inputs a hostile page cannot forge are `Origin`, `Host`, `Sec-Fetch-*` and cookies. `Origin` is always present on browser WebSocket handshakes and includes the port, so an exact `Origin` allowlist check is the dependable defence. `Sec-Fetch-*` is absent on plain-HTTP non-loopback origins, so it can be a second check but never the only one.
4. **DNS rebinding is stopped by validating `Host`, not by the browser.** Chrome (142, WebSockets from 147) and desktop Firefox (153) now put a permission prompt in front of public-to-local requests, which blunts rebinding, but Safari ships nothing equivalent and the prompt is a user-grantable permission, not a guarantee.
5. **Cookies are not isolated by port, and `SameSite` ignores ports.** Any other process serving HTTP on the same hostname at another port (including another OS user's on `localhost`) receives the web monitor's cookies when the browser visits it, counts as same-site, and can overwrite cookies. `HttpOnly`, `SameSite=Strict` and even `__Host-` do not change that. On loopback a cookie alone cannot be the credential boundary.
6. **A browser WebSocket cannot carry an `Authorization` header.** Script controls only the URL and the `Sec-WebSocket-Protocol` list. Credentials other than cookies must travel in the URL, in the subprotocol list, or in the first messages after the handshake.
7. **Non-extractable WebCrypto keys need a secure context and are weaker than they sound.** They stop script exporting the key, not using it; the spec promises nothing about protection on disk; and Safari deletes script-written storage, including IndexedDB, after seven days of Safari use without interaction with the site unless the web view is a Home Screen web app.
8. **WebAuthn never works on an IP address** and needs a secure context. It works on `http://localhost` by specification and on any HTTPS hostname, where the passkey is bound to that hostname.
9. **Loopback is not private to one OS user.** TCP gives the web monitor no peer credentials; any local user can connect, and any local user can bind the port once the web monitor is not holding it, at which point the browser will send that port's cookies to the impostor.

## 1. Secure contexts

### What counts

The Secure Contexts algorithm returns "potentially trustworthy" for `https`/`wss` origins, for hosts in `127.0.0.0/8` or `::1/128`, for `localhost` and `*.localhost` names when the browser guarantees they resolve to loopback, and for `file` origins; everything else is not trustworthy ([Secure Contexts §3.1](https://w3c.github.io/webappsec-secure-contexts/)). MDN restates this: a document is a secure context when delivered over HTTPS or from a loopback address, and a framed document also needs every ancestor to be secure ([MDN: Secure contexts](https://developer.mozilla.org/docs/Web/Security/Defenses/Secure_Contexts)).

Consequences for the four kinds of private origin (*inference* from the algorithm above):

| Origin | Secure context |
| --- | --- |
| `http://127.0.0.1:<port>`, `http://[::1]:<port>`, `http://localhost:<port>` | Yes |
| `http://192.168.x.x:<port>` or any other LAN address | No |
| `http://100.x.y.z:<port>` (tailnet address) or `http://<name>` over a tailnet | No |
| `https://<any hostname or IP>` with a certificate the browser accepts | Yes |

The step that trusts loopback is keyed on the host, not the scheme, so being on a private network confers nothing: only loopback is special.

### Per browser

- **Chrome.** Chrome committed to forcing `localhost` to resolve to loopback so that `http://localhost` and `http://*.localhost` can be secure contexts, as `127.0.0.1` already was ([blink-dev: Treat `http://localhost` as a secure context](https://groups.google.com/a/chromium.org/g/blink-dev/c/RC9dSw-O3fE)).
- **Firefox.** Firefox hardcodes localhost names to loopback ([Bugzilla 1220810](https://bugzilla.mozilla.org/show_bug.cgi?id=1220810)) and treats HTTP `localhost` as secure for cookies since Firefox 75 ([Bugzilla 1648993](https://bugzilla.mozilla.org/show_bug.cgi?id=1648993)).
- **Safari / WebKit.** WebKit's `SecurityOrigin` treats `localhost`, `*.localhost`, `127.*.*.*` and `[::1]` as potentially trustworthy ([WebKit `SecurityOrigin.cpp`](https://webkit.googlesource.com/WebKit/+/master/Source/WebCore/page/SecurityOrigin.cpp)), and Safari reports `isSecureContext` true on localhost ([WebKit bug 232088](https://bugs.webkit.org/show_bug.cgi?id=232088)). But WebKit is internally inconsistent: other code paths treat loopback as insecure for cookies and mixed content, and the tracking bugs are still open ([WebKit bug 281149](https://bugs.webkit.org/show_bug.cgi?id=281149), [WebKit bug 218980](https://bugs.webkit.org/show_bug.cgi?id=218980)).

### What needs a secure context

MDN lists, among others: Service Workers, Web Crypto API, Web Authentication, Credential Management, async Clipboard, Notifications, Push, Storage API, Cookie Store, Web Locks, Screen Wake Lock, Web Share and VirtualKeyboard. WebSocket and IndexedDB are not on the list ([MDN: Features restricted to secure contexts](https://developer.mozilla.org/docs/Web/Security/Secure_Contexts/features_restricted_to_secure_contexts)). In the Web Crypto IDL, `SubtleCrypto`, `CryptoKey` and `crypto.randomUUID()` are `[SecureContext]`; `crypto.getRandomValues()` is not ([Web Cryptography API](https://w3c.github.io/webcrypto)).

So a web view on a non-secure origin can still open a WebSocket, use IndexedDB and get random bytes, but cannot do any key agreement, signing or encryption with the platform's crypto, cannot use passkeys, and cannot keep a phone awake or copy to the clipboard through the async API.

### Mixed content

A request is mixed content when its URL is not potentially trustworthy and the page prohibits mixed contexts ([Mixed Content](https://w3c.github.io/webappsec-mixed-content/)). An HTTPS web view therefore cannot open `ws://` to a non-loopback address (*inference*; the spec does not name WebSockets). WebKit additionally still treats loopback as mixed content in places ([WebKit bug 279249](https://www2.webkit.org/show_bug.cgi?id=279249), [WebKit bug 281149](https://bugs.webkit.org/show_bug.cgi?id=281149)). A web view that talks only to its own origin avoids the whole question.

## 2. What the web monitor can rely on in request headers

### Headers a page cannot forge

`Host`, `Origin`, `Cookie`, `Connection`, `Upgrade` and every `Sec-` prefixed header are forbidden request headers: script cannot set or modify them because the browser keeps control of them ([MDN: Forbidden request header](https://developer.mozilla.org/en-US/docs/Glossary/Forbidden_header_name)). This holds only for browsers. A non-browser program can send any `Origin` it likes, which RFC 6455 says outright: the origin model protects against hostile script in a trusted browser, not against other clients ([RFC 6455 §1.6, §10.1, §10.2](https://datatracker.ietf.org/doc/html/rfc6455)). Header checks therefore defend the user's browser against hostile pages; they do nothing against someone who can reach the port directly. That attacker is stopped only by the credential.

### `Origin`

- `Origin` carries scheme, host and port, and no path ([MDN: Origin](https://developer.mozilla.org/en/docs/Web/HTTP/Headers/Origin)).
- A browser must include `Origin` in every WebSocket handshake, and a server that does not want arbitrary pages connecting should compare it to the origins it expects and answer 403 otherwise ([RFC 6455 §4.1, §10.2](https://datatracker.ietf.org/doc/html/rfc6455)).
- On ordinary HTTP requests browsers send `Origin` on cross-origin requests and on same-origin requests other than `GET` and `HEAD`. It is not sent on same-origin `GET`/`HEAD`, and it can be `null` (sandboxed documents, `data:` documents, cross-origin redirects, some referrer policies) ([MDN: Origin](https://developer.mozilla.org/en/docs/Web/HTTP/Headers/Origin)).

Reliance: on the WebSocket handshake, require `Origin` to be present and exactly equal to the origin the web view was served from; treat absent and `null` as failures. On state-changing HTTP requests, the same check works because those are never same-origin `GET`/`HEAD` (*inference* from the rules above). Because `Origin` includes the port, this check distinguishes the web monitor from another service on the same host at a different port, which cookies and `SameSite` cannot do (section 3).

### WebSocket handshakes get no same-origin protection

The WebSocket constructor takes a URL and a subprotocol list. The handshake is a request with mode `websocket` and credentials mode `include`; redirects are errors; cookies are attached through Fetch ([WHATWG WebSockets](https://websockets.spec.whatwg.org)). There is no CORS preflight and no response the browser withholds: RFC 6455 leaves acceptance entirely to the server's `Origin` check ([RFC 6455 §10.2](https://datatracker.ietf.org/doc/html/rfc6455)). Any page in the user's browser can therefore attempt a handshake to the web monitor, and the browser will attach whatever cookies the cookie rules allow. Rejecting that handshake is the web monitor's job.

### `Sec-Fetch-*`

- The Fetch Metadata headers are set only when the request URL is potentially trustworthy ([Fetch Metadata Request Headers](https://w3c.github.io/webappsec-fetch-metadata/)). They are therefore sent to HTTPS origins and to loopback HTTP origins, and are **absent** on plain HTTP to a LAN or tailnet address (*inference* from that condition and section 1).
- `Sec-Fetch-Site` is `same-origin`, `same-site`, `cross-site` or `none` (user-initiated navigation such as typing the address); if any hop of a redirect chain is cross-site the value is `cross-site` ([Fetch Metadata Request Headers](https://w3c.github.io/webappsec-fetch-metadata/)).
- `Sec-Fetch-Mode` has a dedicated `websocket` value for WebSocket handshakes ([MDN: Sec-Fetch-Mode](https://developer.mozilla.org/docs/Web/HTTP/Reference/Headers/Sec-Fetch-Mode)).
- All three engines ship the headers; MDN marks them widely available since March 2023 ([MDN: Sec-Fetch-Mode](https://developer.mozilla.org/docs/Web/HTTP/Reference/Headers/Sec-Fetch-Mode)), and Safari added them in 16.4 ([WebKit Features in Safari 16.4](https://www.webkit.org/?p=13966)).

Reliance: when present, `Sec-Fetch-Site: same-origin` is a strong statement, and rejecting any present value other than `same-origin` (or `none` for the initial top-level page load) is sound. `same-site` must not be accepted, because same-site ignores ports (section 3). Because the headers are missing on non-loopback plain HTTP and on older browsers, their absence cannot be treated as hostile unless the web monitor only ever serves secure contexts. `Origin` remains the primary check.

### `Host` and DNS rebinding

In a rebinding attack the user visits `http://attacker.example:<port>`, and the attacker's DNS then answers with a loopback or LAN address, so the attacker's page becomes same-origin with the web monitor's port as far as the browser is concerned ([Local Network Access §5.2](https://wicg.github.io/local-network-access)). `Origin` and `Sec-Fetch-Site` do not help: the requests really are same-origin. What does differ (*inference*):

- `Host` is `attacker.example:<port>`, and script cannot change it ([MDN: Forbidden request header](https://developer.mozilla.org/en-US/docs/Glossary/Forbidden_header_name)). A web monitor that answers only for an allowlist of `Host` values it was configured or started with (`127.0.0.1:<port>`, `localhost:<port>`, its LAN or tailnet address, its tunnel hostname) refuses the rebound page.
- Cookies are keyed by host name, so cookies set for the web monitor's real name are not sent to `attacker.example` ([RFC 6265bis §5.1.3, §8.5](https://greenbytes.de/tech/specs/draft-ietf-httpbis-rfc6265bis-22.txt)). The rebound page arrives without the credential.
- Over HTTPS the certificate will not match `attacker.example`, so rebinding fails before any request.

### Private Network Access and Local Network Access

Private Network Access (CORS preflights to private addresses) is on hold; Chrome replaced it with Local Network Access (LNA), a permission prompt ([Chrome: New permission prompt for Local Network Access](https://developer.chrome.com/blog/local-network-access)). The web monitor must not expect or depend on `Access-Control-Request-Private-Network` preflights.

How LNA is specified ([Local Network Access](https://wicg.github.io/local-network-access), a Community Group draft):

- Addresses are classed as loopback (`127.0.0.0/8`, `::1`), local (RFC 1918 ranges, `169.254.0.0/16`, `fc00::/7`, `fe80::/10`, and `100.64.0.0/10`, the range tailnet addresses come from) or public.
- A request is a local network request only when the target is in a *less public* space than the page that makes it. Loopback to loopback is not one, and Chromium does not enforce local to local.
- Such a request needs a user-granted permission (`local-network` or `loopback-network`), available only to secure contexts. The check runs per connection, on the address actually connected to, specifically so that rebinding cannot dodge it.
- The spec says it "merely mitigates attacks against local web services, it cannot fully solve them".

What ships:

| Browser | State |
| --- | --- |
| Chrome | Prompt for `fetch()`, subresources and subframe navigations since Chrome 142 ([Chrome 142 release notes](https://developer.chrome.com/release-notes/142)). WebSocket and WebTransport connections gated since Chrome 147 ([Chrome 147 release notes](https://developer.chrome.com/release-notes/147), [Intent to Ship](https://groups.google.com/a/chromium.org/g/Blink-dev/c/O6GMKt44Ups)). Main-frame navigations are not gated ([Local Network Access](https://wicg.github.io/local-network-access)). Not supported on Android WebView ([Intent to Ship](https://groups.google.com/a/chromium.org/g/Blink-dev/c/O6GMKt44Ups)). |
| Firefox | Desktop Firefox 153 and later prompts before a public site reaches apps on the device or devices on the local network, on by default ([Mozilla Support](https://support.mozilla.org/en-US/kb/control-personal-device-local-network-permissions-firefox)). |
| Safari | Not shipped. The tracking bug is open with enabling by default and the secure-context restriction still outstanding ([WebKit bug 250607](https://bugs.webkit.org/show_bug.cgi?id=250607)); WebKit's standards position is supportive ([WebKit standards-positions #163](https://github.com/WebKit/standards-positions/issues/163)). |

Reliance: none. LNA is defence in depth supplied by two of three engines, the user can grant it to a hostile site, and it does not cover a hostile page that is itself on a local or loopback origin. It also should not obstruct the web view (*inference*): a web view that talks only to its own origin makes loopback-to-loopback, local-to-local or public-to-public requests, none of which are local network requests. A web view loaded from a public tunnel hostname is classed by the tunnel endpoint's public address, because resources fetched through a proxy take the proxy's address ([Local Network Access §4.2](https://wicg.github.io/local-network-access)).

### iOS and macOS local network privacy

Apple's OS-level local network privacy prompts per app before outgoing traffic to a local network address, on iOS 14+ and macOS 15+. Traffic from Safari, `WKWebView` and `SFSafariViewController` is exempt. A "local network" here is one on a broadcast-capable interface such as Wi-Fi or Ethernet, not cellular or VPN ([Apple TN3179](https://developer.apple.com/documentation/technotes/tn3179-understanding-local-network-privacy.md)). So Safari reaches a LAN web monitor without an OS prompt, a tailnet interface is not a local network for this purpose, and the OS feature provides no protection against hostile pages in Safari.

## 3. Cookies

### Scope: host, not origin

- Cookies do not provide isolation by port: a cookie readable by a service on one port is readable by a service on another port of the same host. They also do not isolate by scheme ([RFC 6265bis §8.5](https://greenbytes.de/tech/specs/draft-ietf-httpbis-rfc6265bis-22.txt)).
- Cookies have weak integrity: an active network attacker on a plain-HTTP connection can inject `Set-Cookie` and the server cannot tell injected cookies from its own ([RFC 6265bis §8.6](https://greenbytes.de/tech/specs/draft-ietf-httpbis-rfc6265bis-22.txt)).
- `__Host-` requires `Secure`, `Path=/` and no `Domain`, which pins a cookie to one host, but "ports are the only piece of the origin model that `__Host-` cookies continue to ignore" ([RFC 6265bis §4.1.3.2](https://greenbytes.de/tech/specs/draft-ietf-httpbis-rfc6265bis-22.txt)).

For the web monitor (*inference*): on `localhost` or `127.0.0.1`, every other HTTP service on the machine, on any port, is in the same cookie jar. If the browser makes any request to such a service, it sends the web monitor's cookie to it, `HttpOnly` or not, and that service can set or overwrite the cookie. The same holds for another service on the same LAN address, tailnet name or tunnel hostname. No cookie attribute fixes this; only a distinct host name per web monitor does, or a credential that is not a cookie.

### `SameSite`

- Two origins are same-site when their schemes match and their hosts are equal (when the host has no registrable domain, as with an IP address or `localhost`) or share a registrable domain. "For schemelessly same site and same site, the port and domain components are ignored" ([HTML Standard §7.1.1.1](https://html.spec.whatwg.org/multipage/browsers.html)).
- `Strict` sends the cookie only on same-site requests; `Lax` adds cross-site top-level navigations with safe methods; `None` requires `Secure` ([MDN: Set-Cookie](https://developer.mozilla.org/en-US/Web/HTTP/Headers/Set-Cookie)).
- An unset `SameSite` is treated as `Lax` by Chrome, with a two-minute exception that lets a fresh cookie ride a top-level cross-site `POST` ([Chromium SameSite FAQ](https://www.chromium.org/updates/same-site/faq), [RFC 6265bis §5.6.7.2](https://greenbytes.de/tech/specs/draft-ietf-httpbis-rfc6265bis-22.txt)). MDN says only "some browsers" default to `Lax` ([MDN: Set-Cookie](https://developer.mozilla.org/en-US/Web/HTTP/Headers/Set-Cookie)). Always set the attribute explicitly.
- For WebSockets, a handshake is same-site only when it has the same secureness as the page: `ws://` from an `http://` page or `wss://` from an `https://` page ([web.dev: Schemeful Same-Site](https://web.dev/articles/schemeful-samesite)).
- The cookie spec itself says `SameSite` should not be a site's only CSRF defence ([RFC 6265bis §8.8.1](https://greenbytes.de/tech/specs/draft-ietf-httpbis-rfc6265bis-22.txt)).

For the web monitor (*inference*): `SameSite=Strict` keeps the cookie off handshakes and requests started by pages on other sites, which covers the ordinary hostile public page. It does not cover a page served from another port on the same host, because that page is same-site. On loopback that includes any local user's HTTP service. `Origin` checking closes that gap; `SameSite` does not.

### `Secure` and prefixes on loopback HTTP

- The cookie spec leaves "secure connection" to the browser and notes that most treat `localhost` as trusted ([RFC 6265bis §5.7, §5.8.3](https://greenbytes.de/tech/specs/draft-ietf-httpbis-rfc6265bis-22.txt)).
- Firefox accepts and sends `Secure` cookies on `http://localhost` since Firefox 75 ([Bugzilla 1648993](https://bugzilla.mozilla.org/show_bug.cgi?id=1648993)). Chrome also accepts them ([WebKit bug 232088](https://bugs.webkit.org/show_bug.cgi?id=232088), which records a test where Firefox and Chrome set the cookie and Safari does not).
- Safari rejects `Secure` cookies on `http://localhost`. The bug is open; a commit in April 2026 added part of the support, and a WebKit engineer said the system cookie storage still needs changes before it works ([WebKit bug 232088](https://bugs.webkit.org/show_bug.cgi?id=232088)).
- For prefixes, the spec repository records Firefox accepting `__Host-` and `__Secure-` on `http://localhost`, Chrome accepting `Secure` but rejecting both prefixes there, and Safari rejecting all three ([httpwg/http-extensions #2605](https://github.com/httpwg/http-extensions/issues/2605)).

So there is no single cookie configuration that is both hardened and portable on loopback HTTP: `Secure` and `__Host-` break Safari, and `__Host-` breaks Chrome as last recorded. Over HTTPS all three accept `__Host-…; Secure; HttpOnly; SameSite=Strict; Path=/`.

On plain HTTP to a LAN or tailnet address, `Secure` cookies cannot be set at all, since insecure sites cannot set them ([MDN: Set-Cookie](https://developer.mozilla.org/en-US/Web/HTTP/Headers/Set-Cookie)), and the cookie travels in clear.

### `HttpOnly`

`HttpOnly` keeps the cookie out of script-visible APIs such as `document.cookie`; the browser still attaches it to script-initiated requests ([MDN: Set-Cookie](https://developer.mozilla.org/en-US/Web/HTTP/Headers/Set-Cookie), [RFC 6265bis §4.1.2.6](https://greenbytes.de/tech/specs/draft-ietf-httpbis-rfc6265bis-22.txt)). It limits what injected script can steal, not what it can do while it runs.

### Lifetime on Safari

Safari's tracking prevention deletes all script-writable storage (IndexedDB, LocalStorage, service worker registrations and caches) for a site after seven days of Safari use without user interaction on that site. Web apps added to the Home Screen keep their own counter and are not expected to lose data ([WebKit: Full Third-Party Cookie Blocking and More](https://webkit.org/?p=10218)). The post lists script-writable storage; it does not list cookies set by HTTP response.

## 4. Credentials on a WebSocket handshake

- The browser `WebSocket` constructor is `(url, protocols)`. There is no parameter for request headers, so a web view cannot send `Authorization` or any custom header ([WHATWG WebSockets](https://websockets.spec.whatwg.org)).
- The handshake has credentials mode `include`, so cookies are attached automatically, subject to the cookie rules in section 3 ([WHATWG WebSockets](https://websockets.spec.whatwg.org)).
- RFC 6455 prescribes no client authentication; a server may use whatever a generic HTTP server can: cookies, HTTP authentication or TLS authentication ([RFC 6455 §10.5](https://datatracker.ietf.org/doc/html/rfc6455)).
- What script does control: the URL path and query (fragments are rejected), and the list sent as `Sec-WebSocket-Protocol`. If the page requests subprotocols and the server acknowledges none of them, the connection fails ([WHATWG WebSockets](https://websockets.spec.whatwg.org)).
- The constructor also accepts `http:` and `https:` URLs and relative URLs, converting them to `ws:` and `wss:` ([WHATWG WebSockets](https://websockets.spec.whatwg.org)).

The options for a non-cookie credential are therefore (*inference*): a value in the URL (which lands in logs and history-like surfaces of intermediaries), a value in the subprotocol list (which the server must echo a member of), or an authentication exchange in the first messages after the handshake completes. The last is the only one that can be a proof (a signature or MAC over a server challenge) rather than a bearer value, and the only one that keeps the credential out of the request line and headers.

## 5. Non-extractable WebCrypto keys and WebAuthn

### WebCrypto

- `SubtleCrypto` and `CryptoKey` exist only in secure contexts ([Web Cryptography API](https://w3c.github.io/webcrypto); [MDN: CryptoKey.extractable](https://developer.mozilla.org/docs/Web/API/CryptoKey/extractable)). On plain HTTP to a LAN or tailnet address the web view has no platform crypto at all.
- `extractable: false` means `exportKey()` and `wrapKey()` throw ([MDN: CryptoKey.extractable](https://developer.mozilla.org/docs/Web/API/CryptoKey/extractable)).
- `CryptoKey` objects are serializable and the spec expects authors to keep them in IndexedDB; otherwise a key lasts only for the current page ([Web Cryptography API](https://w3c.github.io/webcrypto)).
- The spec does not guarantee that key material is not persisted to disk, possibly unencrypted, and says nothing about hardware-backed storage ([Web Cryptography API](https://w3c.github.io/webcrypto)).
- The spec warns that injected script may exfiltrate keys or data ([Web Cryptography API](https://w3c.github.io/webcrypto)). For a non-extractable key the remaining exposure (*inference*) is use, not export: script running in the origin can ask the key to sign or decrypt for as long as it runs.
- IndexedDB is origin-scoped (scheme, host and port), unlike cookies, so a key stored there is not visible to a service on another port (*inference* from the same-origin definition in [HTML Standard §7.1.1](https://html.spec.whatwg.org/multipage/browsers.html)).
- On Safari, IndexedDB falls under the seven-day deletion described in section 3 unless the web view is a Home Screen web app ([WebKit: Full Third-Party Cookie Blocking and More](https://webkit.org/?p=10218)).

A non-extractable key in IndexedDB is thus a credential that cannot be copied out of the browser by script and is not shared across ports, which is better than a cookie on both counts. It is not protected from the device's own user or OS, which the threat model already excludes, and on Safari it can vanish.

### WebAuthn and passkeys

- The API is exposed only in secure contexts ([Web Authentication Level 3](https://www.w3.org/TR/webauthn-3/)).
- The page's effective domain must be a valid domain, or the call throws `SecurityError`: "Only the domain format of host is allowed here." IP addresses, IPv4 or IPv6, are excluded ([Web Authentication Level 3](https://www.w3.org/TR/webauthn-3/)).
- The RP ID must equal the origin's effective domain or be a registrable domain suffix of it, and the origin must be `https`, or `http` with host `localhost`. The port is unrestricted ([Web Authentication Level 3](https://www.w3.org/TR/webauthn-3/)).
- Firefox's implementation refuses WebAuthn for any principal that is an IP address and otherwise exposes it in all secure contexts ([Firefox `WebAuthnUtil.cpp`](https://arai.searchfox.org/firefox-main/source/dom/webauthn/WebAuthnUtil.cpp)).

By origin (*inference*): `http://localhost:<port>` can use WebAuthn; `http://127.0.0.1:<port>` cannot; no LAN or tailnet IP can, with or without HTTPS; an HTTPS hostname (a tailnet machine name with a certificate, or a tunnel hostname) can, with the passkey bound to that hostname. Because the RP ID ignores the port, a passkey for `localhost` is usable by any service on any `localhost` port, including another local user's, so on loopback a passkey identifies the user to "something on this machine", not to the web monitor. A tunnel hostname that changes between runs strands its passkeys.

## 6. Other local OS users and a loopback port

- A loopback TCP port has no per-user access control. `ip(7)` describes binding and the loopback address and no restriction on who may connect ([ip(7)](https://www.man7.org/linux/man-pages/man7/ip.7.html)); `socket(7)` documents peer credentials (`SO_PEERCRED`) by reference to Unix domain sockets ([socket(7)](https://manpages.debian.org/unstable/manpages/socket.7.en.html)). The web monitor cannot learn the UID of a TCP peer the way a session daemon does on its private socket, so same-UID trust (ADR 0003) does not carry over (*inference*).
- Only one socket may be bound to a given local address and port. Ports below 1024 need `CAP_NET_BIND_SERVICE`; any user may bind the rest ([ip(7)](https://www.man7.org/linux/man-pages/man7/ip.7.html)).
- `SO_REUSEPORT` lets several sockets share an address, but "to prevent port hijacking, all of the processes binding to the same address must have the same effective UID". With `SO_REUSEADDR`, a bind still fails while an active listening socket holds the address ([socket(7)](https://manpages.debian.org/unstable/manpages/socket.7.en.html)). On Linux another user therefore cannot share or take a port the web monitor is listening on.

What another local user can do (*inference* from the above plus sections 2 and 3):

1. **Reach the port.** Connect to the web monitor's loopback port directly, with any headers. Only the credential stops them.
2. **Impersonate the port when it is free.** Before the web monitor starts, after it exits, or if it restarts on a different port, bind the old port and serve their own page. The browser treats it as the same origin: it sends the origin's cookies, and exposes the origin's IndexedDB (including any stored key, usable though not exportable), service worker and saved permissions to the impostor's script. Plain HTTP gives the browser no way to tell the two processes apart. Only a server identity the impostor cannot present does: a TLS key held in the user's private files, or an application-level proof.
3. **Collect cookies from a neighbouring port.** Run a service on another loopback port and get the user's browser to request it. Cookies for `localhost` or `127.0.0.1` are sent to it regardless of port (section 3).
4. **Act as a same-site page.** From that neighbouring port, open a WebSocket to the web monitor. `SameSite=Strict` cookies are attached because the page is same-site. The `Origin` header shows the other port, so an exact `Origin` check rejects it.

## Consequences the specification has to absorb

These are not decisions; they are the constraints the sources impose.

- On every WebSocket handshake and every state-changing request, require `Host` in an allowlist and `Origin` exactly equal to the web view's origin. Use `Sec-Fetch-Site: same-origin` as an extra check when the header is present.
- Do not treat LNA, PNA, OS local network privacy or `SameSite` as a boundary.
- A cookie is at most a convenience on loopback and plain HTTP: it is shared across ports, sent to port impostors, and cannot be `Secure` or `__Host-` portably. A credential that proves possession inside the WebSocket (a non-extractable key answering a challenge) avoids the port-sharing and bearer problems but needs a secure context, and on Safari needs a plan for storage deletion.
- A phone on a LAN or tailnet gets a secure context only through HTTPS, which also defeats rebinding and port impersonation if the impostor cannot obtain the key. That is the known hard problem in the map: whoever terminates TLS serves the script.
- WebAuthn is available only on `localhost` and on HTTPS hostnames, never on IP addresses, and on loopback it does not distinguish the web monitor from other local services.

## Unverified

- **Safari and Chrome WebAuthn on `http://localhost`.** The specification permits it; I did not find vendor documentation or source confirming Safari's or Chrome's behaviour. Firefox's source was checked.
- **Chrome and Safari rejecting IP-address RP IDs.** A 2020 report to the WebAuthn list said Firefox and Edge accepted IPv4 RP IDs and Chrome did not ([public-webauthn](https://lists.w3.org/Archives/Public/public-webauthn/2020Jan/0033.html)); current Firefox source rejects IP principals. Current Chrome and Safari behaviour was not confirmed from source.
- **Current Chrome handling of `__Host-`/`__Secure-` on `http://localhost`.** The source is an issue thread, not release documentation, and Chrome had the change in scope.
- **Which Chrome release began accepting `Secure` cookies on `http://localhost`.** Behaviour is confirmed by tests in a WebKit bug; older MDN text gave Chrome 89, which I could not trace to a Chromium source.
- **Whether Firefox release defaults unset `SameSite` to `Lax`, and whether Safari does.** Could not read the Mozilla tracking bug ([1617609](https://bugzilla.mozilla.org/show_bug.cgi?id=1617609)); MDN's compatibility table did not load.
- **Schemeful same-site shipping status** in release Chrome, Firefox and Safari. The web.dev article describes it behind flags.
- **`SameSite` on WebSocket handshakes in Safari and Firefox.** The same-secureness rule is documented by Chrome's team only.
- **`Sec-Fetch-*` on WebSocket handshakes in Safari, and on loopback HTTP in each browser.** The specification says they are sent; not tested or confirmed per browser.
- **Safari resolving `*.localhost` to loopback.** WebKit treats such names as potentially trustworthy; whether the system resolver maps arbitrary `*.localhost` names to loopback on macOS and iOS was not confirmed. This matters if a distinct host name per web monitor is used to escape the shared cookie jar.
- **Whether Safari's seven-day deletion ever applies to `HttpOnly` cookies set by response header**, and how tracking prevention treats IP-address and `localhost` sites.
- **Chrome and Firefox on iOS.** They use WebKit, so Safari's behaviour should apply, and Apple exempts `WKWebView` traffic from local network privacy; I did not confirm that these apps make no local-network requests outside `WKWebView` that would trigger the OS prompt. Browsers with alternative engines in some regions were not examined.
- **Firefox for Android and LNA.** The Mozilla article covers desktop only.
- **Whether Firefox's LNA covers WebSockets.** Not stated in the support article.
- **Public Suffix List membership of tailnet and tunnel domains** (`ts.net`, `trycloudflare.com`, `ngrok` domains, dev tunnels). If a tunnel's parent domain is not a public suffix, other tenants' hostnames are same-site with the web view and can set cookies for the shared parent domain. Could not fetch the list.
- **URL userinfo on WebSocket URLs** (`ws://user:pass@host`) as a way to trigger HTTP authentication. The WHATWG text is silent and no vendor source was found.
- **macOS and BSD bind semantics.** The port-sharing rules cited are from Linux man pages. Whether macOS lets another user bind a wildcard or more specific address on a port the web monitor holds on `127.0.0.1` was not confirmed from Apple documentation. The equivalent Linux case (wildcard bind while a loopback-only listener exists) is also not spelled out in the pages read.
- **An explicit primary statement that any local user may connect to a loopback TCP port.** The claim rests on the absence of any access control in `ip(7)`, not on a sentence that says so.
- **Mixed content rules for `ws://` specifically.** Inferred from the general mixed-content definition.
