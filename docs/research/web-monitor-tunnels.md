# How Tailscale serve, dev tunnels, ngrok and Cloudflare Tunnel carry a WebSocket terminal

Research for [Establish how Tailscale serve, dev tunnels and ngrok carry a WebSocket terminal](https://github.com/jackgeek/climon2/issues/243),
a child of [Chart the web monitor specification](https://github.com/jackgeek/climon2/issues/229).

Researched 2026-11-14. This file reports facts and decides no policy. It
reuses, and does not repeat, the TLS findings in
[web-monitor-tls.md](https://github.com/jackgeek/climon2/blob/research/web-monitor-tls/docs/research/web-monitor-tls.md)
(cited as [TLS]) and the browser findings in
[web-monitor-browser.md](https://github.com/jackgeek/climon2/blob/research/web-monitor-browser/docs/research/web-monitor-browser.md)
(cited as [BRW]).

Every claim cites a primary source from the list at the end: the vendor's own
documentation, or the vendor's own source code at a pinned commit.
**Inference** marks a conclusion that follows from a cited statement but is
not stated by the source. **Unverified** marks something no primary source
confirmed; those are collected in the last section. Nothing here was run:
no tunnel was started and no header was observed on the wire.

Setting used throughout: the *web monitor* listens on loopback (or a Unix
domain socket) on the serving machine S and speaks plain HTTP. The *agent* is
the transport's local process on S (`tailscaled`, `devtunnel`, `ngrok`,
`cloudflared`) that connects to the web monitor. The *web view* runs in a
browser on another device.

## Summary

1. **All four transports pass WebSocket upgrades without configuration.**
   Tailscale's proxy is Go's `ReverseProxy`, which forwards upgrades [T5][G1];
   dev tunnels list WS(S) as a web-forwarding protocol [D1]; ngrok and
   Cloudflare say WebSockets work with no configuration [N1][C1].
2. **None documents a maximum WebSocket duration, and three can drop an idle
   or long-lived connection.** ngrok lists 5 minute idle timeouts [N1].
   Cloudflare closes a WebSocket idle "for a period of time" and may
   terminate WebSockets when it releases new code [C1]. Dev tunnels meter
   bandwidth per user per month [D4]. Tailscale's proxy sets no timeout on
   the proxied connection (**Inference** from [T5]). The web view must
   therefore send its own heartbeat and reconnect on every transport.
3. **Dev tunnels rewrite `Host` and `Origin` to localhost by default.** The
   dev tunnels contract says web forwarding rewrites `Host` to `localhost`
   and `Origin` to `http(s)://localhost` unless told otherwise [D5]. Under
   the default, the web monitor's exact `Origin` and `Host` checks see
   localhost values, not the browser's real ones (**Inference**). Whether a
   hostile page's `Origin` is rewritten the same way is **Unverified**, and
   it decides whether the `Origin` check survives a dev tunnel at all.
4. **The other three keep the external hostname visible.** Tailscale passes
   the incoming `Host` to a loopback TCP target and always sets
   `X-Forwarded-Host` [T5]. ngrok sets `X-Forwarded-Host` [N1]. `cloudflared`
   leaves `Host` alone unless `httpHostHeader` is set [C5][C4].
5. **Only `tailscale serve` is private by default without a login page from
   a third party.** Serve reaches the tailnet only [T2]. Dev tunnels are
   private to the creating account [D1]. ngrok endpoints and Cloudflare quick
   tunnels are public URLs unless the user adds authentication [N1][C3].
   Funnel is public by design [T4].
6. **`tailscaled` and `cloudflared` can target a Unix domain socket; ngrok
   and dev tunnels document only TCP targets.** With a Unix socket target
   Tailscale sends `Host: localhost` and the external name only in
   `X-Forwarded-Host` [T5]. Configuring it needs root [T2][T6].
7. **Two transports put their own page in front of the web view.** Dev
   tunnels show an anti-phishing interstitial on a first browser visit [D1].
   ngrok's free plan shows an interstitial before all HTML browser traffic
   [N3]. Neither is documented as altering the page once the user continues.
8. **Headers a transport adds are only as trustworthy as the loopback
   port.** Any local process that can connect to the web monitor's port can
   send the same `X-Forwarded-*` or `Tailscale-User-*` headers; Tailscale
   says so itself [T2]. A Unix socket with private permissions narrows who
   can connect (**Inference**).

## Comparison

| | `tailscale serve` | Tailscale Funnel | Dev tunnels | ngrok | Cloudflare Tunnel |
|---|---|---|---|---|---|
| WebSocket upgrade | Forwarded by Go `ReverseProxy` [T5][G1] | Same proxy path [T5] | WS(S) supported on web forwarding [D1] | "supported out-of-the-box" [N1] | Supported with no configuration [C1]; `cloudflared` has an explicit WebSocket path [C6] |
| Idle or duration limits | None set in the proxy (**Inference** [T5]) | Non-configurable bandwidth limits [T4] | 5 GB per user per month, 1000 active connections and 1500 requests/min per port, 20 MB/s per tunnel [D4]; idle limit **Unverified** | Client and server idle timeouts 5 min; no round-trip timeout [N1]; free plan 1 GB out and 20,000 requests per month [N3] | Closed when idle for an unstated period; code releases terminate WebSockets [C1]; quick tunnels: 200 in-flight requests [C3] |
| `Host` the web monitor sees | The external name, for a TCP target; `localhost` for a Unix socket target [T5] | Same [T5] | `localhost` by default; the tunnel hostname if `IsHostHeaderUnchanged` [D5] | **Unverified**; rewriting is described as opt-in [N4] | The public hostname unless `httpHostHeader` is set (**Inference** [C5][C4]) |
| `Origin` the web monitor sees | Not touched by the proxy (**Inference** [T5][G1]) | Same | `http(s)://localhost` by default; the tunnel origin if `IsOriginHeaderUnchanged` [D5] | **Unverified** | Not touched in the code read (**Inference** [C5][C6]) |
| Forwarded headers | `X-Forwarded-Host`, `X-Forwarded-Proto`, `X-Forwarded-For`, `Tailscale-User-*`, `Tailscale-Headers-Info` [T5] | `X-Forwarded-*` and `Tailscale-Funnel-Request: ?1`; no identity headers [T5][T2] | None documented | `X-Forwarded-For`, `X-Forwarded-Proto`, `X-Forwarded-Host` [N1] | `CF-Connecting-IP`, `X-Forwarded-For`, `X-Forwarded-Proto`, `CF-Ray`, `CF-IPCountry`, `CF-Visitor`, `CDN-Loop` [C7]; `X-Forwarded-Host` only when `httpHostHeader` is set [C5] |
| Learning the external hostname | `X-Forwarded-Host` per request [T5]; `DNSName` in status [T7]; CLI prints the URL [T6] | Same | `devtunnel host` prints the URL [D2]; not in the request by default [D5] | `X-Forwarded-Host` [N1]; agent API `GET /api/endpoints` on `127.0.0.1:4040`, no authentication [N2] | `Host` (**Inference** [C5]); quick tunnel URL is logged by `cloudflared` [C8] |
| Default access control | Tailnet only, subject to the tailnet's access rules [T2] | Public internet [T4] | Private to the account that created the tunnel [D1] | Public unless secured [N1] | Quick tunnel: public unless `--allowed-mail` [C3]; named tunnel default **Unverified** |
| How access is changed | Tailnet policy [T2]; switch to Funnel for public [T4] | `funnel` node attribute, default all members [T4] | `--allow-anonymous`, `--tenant`, `--organization`, access tokens [D1][D2] | Traffic Policy: basic auth, OAuth, OIDC, IP restriction, mutual TLS and others [N1] | `--allowed-mail` on quick tunnels [C3]; `access` origin parameter validates the Access JWT [C4] |
| TLS terminates at | `tailscaled` on S [TLS] | `tailscaled` on S; relays do not decrypt [T4] | Microsoft's service ingress [D1] | ngrok's cloud service for `https` endpoints [N1] | Cloudflare (**Inference** [C7]) |
| Unix domain socket target | Yes: `unix:/path`, Linux, macOS and BSD, needs root to configure [T6][T8][T2] | Same code path [T5] | Not documented; only ports [D2] | Not documented for the agent [N5]; the Go SDK accepts a custom dialer [N6] | Yes: `unix:` and `unix+tls:` services, `--unix-socket` flag [C9][C2] |
| Interstitial or injected content | None in the proxy code read (**Inference** [T5]) | Same | Anti-phishing interstitial on first HTML `GET`; sign-in for private tunnels [D1] | Free plan: interstitial before HTML browser traffic, suppressed 7 days by cookie [N3] | None documented for tunnels; zone features **Unverified** |

## 1. `tailscale serve`

Source references are to `tailscale/tailscale` at commit `9d53136` (main,
version file 1.105.0). Which released client first contains each behaviour is
**Unverified**.

### WebSocket and limits

- Serve builds an `httputil.ReverseProxy` with a `Rewrite` function for every
  proxied request [T5]. Go documents that this proxy forwards upgrade
  requests and, after a `101 Switching Protocols` response, forwards data in
  both directions [G1].
- Go's proxy rejects an `Upgrade` header that arrives on a connection that is
  not HTTP/1 [G1]. `tailscaled` offers both `h2` and `http/1.1` in TLS [T9].
  **Inference**: a WebSocket through serve works only on an HTTP/1.1
  connection. Whether every target browser opens one for `wss:` here is
  **Unverified**.
- The `http.Server` that serve creates sets no read, write or idle timeout,
  and the proxy transport's only timeouts are for idle pooled connections
  (90 s), the TLS handshake and `Expect: 100-continue` [T5]. **Inference**:
  the proxy itself imposes no idle or duration limit on an upgraded
  connection. Serve documentation states none either [T2].
- Stopping serve closes the proxy's idle backend connections [T5]. The
  effect on an established WebSocket was not traced (**Unverified**).

### Headers

For a loopback TCP target, the proxy sets the outgoing `Host` to the incoming
`Host` [T5]. For a Unix socket target it sets `Host` to `localhost` [T5]. In
both cases it then sets [T5]:

- `X-Forwarded-Host` to the incoming `Host`;
- `X-Forwarded-Proto: https` when the incoming connection used TLS;
- `X-Forwarded-For` to the source address of the connection.

Go removes any `Forwarded`, `X-Forwarded-For`, `X-Forwarded-Host` and
`X-Forwarded-Proto` headers the client sent before `Rewrite` runs [G1], so a
browser cannot supply those values through serve (**Inference**).

Serve deletes any incoming `Tailscale-User-Login`, `Tailscale-User-Name`,
`Tailscale-User-Profile-Pic`, `Tailscale-Funnel-Request` and
`Tailscale-Headers-Info` headers, then adds the three identity headers and
`Tailscale-Headers-Info` for a tailnet user's device [T5][T2]. It adds none
for a tagged device [T5][T2].

The proxy code does not read or write `Origin` [T5], and Go's proxy copies
other request headers through [G1]. **Inference**: the web monitor sees the
browser's own `Origin`.

Tailscale warns that these headers prove nothing if another process can reach
the backend directly, and advises that the backend listen only on localhost
[T2]. Loopback TCP carries no peer identity [BRW], so any local OS user can
connect to the web monitor's port and send the same headers (**Inference**).

### Learning the external hostname

- Per request: `X-Forwarded-Host`, and `Host` for a TCP target [T5].
- From the CLI: `tailscale serve` prints the URL under "Available within your
  tailnet:" [T6], and `tailscale serve status --json` exists, though it
  returns different information from the plain form [T3].
- From status: the node's status record has a `DNSName` field, its FQDN with
  a trailing dot [T7].

### Access control

Serve shares with the tailnet only, DNS names are limited to the tailnet's
domain, and the tailnet's access rules apply to it as to any service [T2].
HTTPS certificates must be enabled in the tailnet [T2]. TLS terminates in
`tailscaled` on S [TLS].

### Unix domain socket

- `tailscale serve unix:/path` is accepted on Linux, macOS and BSD and
  refused on Windows [T6][T8]. The documentation pages read do not list it
  as a target type; the CLI help text and the source do [T3][T6].
- Configuring a Unix socket target requires root [T2][T6].
- `tailscaled` dials the socket itself [T5]. **Inference**: the socket's
  permissions must admit the user `tailscaled` runs as, normally root.
- Serve refuses to proxy to `tailscaled`'s own socket, and on Linux pins the
  socket's inode before connecting so that swapping the path cannot retarget
  the dial; other platforms dial the path directly [T5][T10].
- With a Unix socket target the web monitor sees `Host: localhost` and must
  read the external name from `X-Forwarded-Host` [T5].

### Injected content

The proxy path read adds headers only; nothing in it alters a response body
[T5]. Serve documentation mentions no interstitial [T2].

## 2. Tailscale Funnel

- Funnel uses the same handler and reverse proxy as serve, with a marker for
  the funnelled connection [T5]. WebSocket behaviour, `Host` handling and
  Unix socket targets are therefore the same (**Inference**).
- For a funnelled request serve sets `Tailscale-Funnel-Request: ?1` and no
  identity headers [T5]; the documentation confirms Funnel traffic carries no
  identity headers [T2]. The header lets the web monitor tell public traffic
  from tailnet traffic, subject to the loopback caveat above (**Inference**).
- Funnel publishes the resource to the internet for anyone, with or without
  Tailscale [T4]. It needs the `funnel` node attribute in the tailnet policy,
  which by default allows every member [T4].
- Funnel listens only on ports 443, 8443 and 10000, uses only the tailnet's
  DNS names, and is subject to non-configurable bandwidth limits [T4]. The
  limit values are not published (**Unverified**).
- Funnel relays do not decrypt; `tailscaled` on S terminates TLS [T4].
- One port cannot be used for serve and Funnel at once [T2][T4].

## 3. Microsoft dev tunnels

The `devtunnel` CLI is closed source; the SDK and its contracts are public.
Source references are to `microsoft/dev-tunnels` at commit `37956c6`.

### WebSocket and limits

- Tunnel ports using HTTP(S)/WS(S) are reachable at a web-forwarding URL of
  the form `https://<tunnelid>-<port>.devtunnels.ms`; insecure connections are
  upgraded to HTTPS/WSS [D1].
- Published limits, reset monthly: 5 GB bandwidth per user, 10 tunnels per
  user, 1000 active connections per port, 10 ports per tunnel, 1500 HTTP
  requests per minute per port, up to 20 MB/s per tunnel, and a 16 MB maximum
  web-forwarding request body [D4].
- No idle or maximum duration for a WebSocket is documented (**Unverified**).
- The service is in public preview with no service-level agreement and is
  described as not for production workloads [D2][D3].
- A tunnel hosted without an ID is temporary and deleted when the host
  process exits; expiration can be set between 1 hour and 30 days [D2]. The
  default expiration is not stated (**Unverified**).

### Headers

- After terminating TLS, the service rewrites headers [D1]. The security page
  does not say which.
- The tunnel contract says: with `HostHeader` unset, web forwarding rewrites
  `Host` to `localhost`; with `IsHostHeaderUnchanged` true, `Host` stays the
  web-forwarding host, for example `tunnel-name-8080.devtunnels.ms` [D5].
- The same contract says: with `OriginHeader` unset, web forwarding rewrites
  `Origin` to `http(s)://localhost`; with `IsOriginHeaderUnchanged` true,
  `Origin` stays the tunnel's web-forwarding origin [D5]. Both options exist
  at tunnel and port level, and the port level wins [D5].
- **Inference**: under the defaults the web monitor cannot learn the external
  hostname from a request, and an exact check of `Origin` and `Host` against
  the external name fails unless these options are set.
- The contract does not say whether rewriting applies to every `Origin` value
  or only to the tunnel's own origin. If every value is rewritten, a
  cross-site request arrives looking same-origin (**Unverified**; see the
  last section).
- No `X-Forwarded-*` header is documented or named in the SDK contracts
  (**Unverified** whether any is sent).
- The CLI flags that set these options are not in the command reference [D2]
  (**Unverified**).

### Learning the external hostname

`devtunnel host` prints a line such as `Hosting port 3000 at
https://<id>-3000.<cluster>.devtunnels.ms/` [D2]. The endpoint contract
carries a `PortUriFormat` from which the SDK builds the same URL [D6]. No
machine-readable CLI output is documented [D2] (**Unverified**).

### Access control

- By default a tunnel and its ports are private: hosting and connecting need
  the same Microsoft, Entra ID or GitHub account that created the tunnel
  [D1]. **Inference**: the browser on the other device must sign in to that
  account before any request reaches the web monitor.
- `--allow-anonymous` opens a tunnel or port to anyone who has the URL;
  `--tenant` and `--organization` extend access to an Entra tenant or a
  GitHub organisation [D1][D2].
- Access tokens scoped to one tunnel can be issued, currently expire after 24
  hours, and are presented in `X-Tunnel-Authorization: tunnel <TOKEN>` [D1].
- TLS terminates at the service ingress with Microsoft's certificates [D1];
  see [TLS] for the consequence.

### Unix domain socket

The CLI documents only local port numbers as targets [D2]. No Unix socket
target is documented (**Unverified** that none exists).

### Injected content

A first visit to a web-forwarding URL shows an anti-phishing interstitial
page. It is skipped when the method is not `GET`, when `Accept` lacks
`text/html`, when the request carries `X-Tunnel-Skip-AntiPhishing-Page` or
`X-Tunnel-Authorization`, or once the user has clicked continue [D1].
**Inference**: a WebSocket handshake or a script `fetch` made before the user
has clicked through is not shown the page, because it does not ask for
`text/html`; the first navigation is. HSTS is set with a one-year max-age
[D1].

## 4. ngrok

### WebSocket and limits

- WebSocket connections work with no configuration; ngrok forwards no
  hop-by-hop headers except `Connection: upgrade` [N1].
- HTTP endpoints list a 5 minute client idle timeout and a 5 minute server
  idle timeout, no round-trip, request or response timeout, no body size
  limit and a 1 MB header limit; changing them means contacting support
  [N1]. Whether the idle timeouts apply after a WebSocket upgrade is not
  stated (**Unverified**).
- Free plan: 1 GB data transfer out and 20,000 HTTP requests per month, up to
  3 online endpoints, no endpoint timeout [N3].

### Headers

- ngrok adds `X-Forwarded-For`, `X-Forwarded-Proto` and `X-Forwarded-Host`.
  The first two are appended to existing values, so the last value is ngrok's
  [N1]. `X-Forwarded-Host` keeps the client's own value if the client sent
  one, and otherwise takes the request's `Host` [N1]. **Inference**: a client
  can choose the `X-Forwarded-Host` the web monitor sees, so it is not
  evidence of the external hostname.
- Rewriting `Host` is described as something the user asks for, with
  `--host-header` or a Traffic Policy action [N4][N1]. The default `Host`
  sent upstream is not stated in the pages read (**Unverified**).
- `Origin` is not mentioned in the pages read (**Unverified**).

### Learning the external hostname

- Every account has one assigned dev domain that does not change between
  runs, and an endpoint with no URL uses it [N1][N3].
- The agent serves a local API at `http://127.0.0.1:4040/api` with no
  authentication; `GET /api/endpoints` lists running endpoints with their
  `url` [N2]. **Inference**: any local process can read it, and the web
  monitor cannot treat the answer as coming from the user.

### Access control

A public endpoint is reachable from the internet unless the user secures it;
the options are Traffic Policy actions including basic auth, OAuth, OpenID
Connect, JWT validation, IP restriction and mutual TLS [N1]. TLS for an
`https` endpoint terminates at ngrok's cloud service; a TLS endpoint lets the
agent or upstream terminate instead [N1].

### Unix domain socket

The agent's `upstream.url` accepts a port, a host and port or an `http`,
`https` or `tcp` URL [N5]. No Unix socket form is documented. The Go SDK lets
a program supply its own dialer for the upstream [N6]; that is a library
integration, which the map places out of scope.

### Injected content

On the free plan ngrok shows an interstitial page in front of all HTML
browser traffic. Clicking through sets a cookie that suppresses it for that
domain for 7 days. A request with the `ngrok-skip-browser-warning` header or
a non-browser `User-Agent` skips it, and paid plans remove it [N3][N7]. When
`Host` rewriting is on, ngrok also rewrites the `Location` response header
[N1].

## 5. Cloudflare Tunnel

Source references are to `cloudflare/cloudflared` at commit `b0297f0`.

### WebSocket and limits

- Cloudflare proxies WebSocket connections on every plan with no
  configuration [C1]. `cloudflared` re-adds `Connection: Upgrade`,
  `Upgrade: websocket` and `Sec-WebSocket-Version: 13` on the request to the
  origin [C6].
- Cloudflare closes a WebSocket when no data moves in either direction for a
  period it does not state, and recommends a heartbeat [C1]. Its general
  limits page lists a 900 s proxy idle timeout; that page has no WebSocket
  row [C10].
- Restarting servers for a code release terminates WebSocket connections
  [C1]. A `cloudflared` configuration update that stops an instance drops
  long-lived requests, WebSockets included [C2].
- Quick tunnels allow 200 in-flight requests, answer further requests with
  `429`, do not support Server-Sent Events and have no uptime guarantee [C3].
  Whether one WebSocket counts as an in-flight request for its whole life is
  **Unverified**.

### Headers

- Cloudflare adds `CF-Connecting-IP`, `X-Forwarded-For`, `X-Forwarded-Proto`
  (overwriting a client value), `CF-Ray`, `CF-IPCountry`, `CF-Visitor` and
  `CDN-Loop`, and sets `Connection` to `Keep-Alive` [C7].
- `cloudflared` changes `Host` only when `httpHostHeader` is set, and in that
  case passes the original as `X-Forwarded-Host` [C5][C4]. **Inference**: by
  default the web monitor sees the public hostname in `Host` and no
  `X-Forwarded-Host`.
- The `cloudflared` code read does not touch `Origin` [C5][C6]
  (**Inference** that it arrives unchanged).

### Learning the external hostname

A named tunnel's hostname is chosen by the user in configuration [C2]. A
quick tunnel's `trycloudflare.com` hostname changes on every run [C3] and
`cloudflared` logs it at start [C8]. A machine-readable way to read it was
not found (**Unverified**).

### Access control

- A quick tunnel needs no account and gives a public URL; `--allowed-mail`
  puts a one-time PIN by email in front of it [C3].
- For named tunnels, the `access` origin parameter makes `cloudflared`
  validate the Cloudflare Access JWT before proxying [C4]. The default
  reachability of a published named tunnel was not confirmed from the pages
  read (**Unverified**).
- Cloudflare reads and rewrites request headers [C7]. **Inference**: TLS
  terminates at Cloudflare, which can read traffic and alter the page script,
  the same tier as dev tunnels and ngrok in the map's transport trust note.

### Unix domain socket

`cloudflared` accepts `unix:/path` and `unix+tls:/path` as ingress services
and a `--unix-socket` flag [C9]; the documentation lists Unix sockets among
supported origins [C2].

### Injected content

No interstitial is documented for tunnels. With `--allowed-mail` or Access,
Cloudflare's own login page precedes the web view [C3][C4]. Zone-level
features that modify HTML were not researched (**Unverified**).

## Unverified

- **Dev tunnels `Origin` rewriting scope.** Whether every incoming `Origin`
  is rewritten to `http(s)://localhost`, or only the tunnel's own origin.
  The answer decides whether the web monitor can detect cross-site WebSocket
  hijacking behind a dev tunnel with default settings.
- Whether dev tunnels send any `X-Forwarded-*` or original-host header.
- The `devtunnel` CLI flags for `HostHeader`, `IsHostHeaderUnchanged`,
  `OriginHeader` and `IsOriginHeaderUnchanged`; the CLI is closed source and
  its reference omits them.
- Dev tunnels: any idle or maximum WebSocket duration, the default tunnel
  expiration, a Unix socket target, and machine-readable CLI output.
- ngrok: the default `Host` and `Origin` sent upstream, whether the 5 minute
  idle timeouts apply to an upgraded WebSocket, and whether the agent accepts
  a Unix socket upstream undocumented.
- Tailscale: which released client first accepts `unix:` targets; whether
  browsers always use HTTP/1.1 for `wss:` to `tailscaled`; what happens to an
  established WebSocket when serve configuration changes; which `Host`
  values other than the full tailnet name reach a serve handler; Funnel's
  bandwidth limit values.
- Cloudflare: the idle period after which a WebSocket is closed; whether a
  WebSocket holds a quick tunnel in-flight slot; default reachability of a
  published named tunnel; a machine-readable quick tunnel URL; zone features
  that modify HTML; where exactly TLS terminates, stated by Cloudflare.
- Nothing was run. Every header claim comes from documentation or source
  reading, not from a captured request.
- Several documentation pages were read through a fetch tool that returns
  extracts. Quoted values were taken from those extracts.

## Sources

Tailscale:

- [T2] "Tailscale Serve": <https://tailscale.com/kb/1312/serve>
- [T3] "tailscale serve command": <https://tailscale.com/docs/reference/tailscale-cli/serve>
- [T4] "Tailscale Funnel": <https://tailscale.com/kb/1223/funnel>
- [T5] `ipn/ipnlocal/serve.go` (`tcpHandlerForServeTCP`, `proxyHandlerForBackend`, `reverseProxy.ServeHTTP`, `getTransport`, `addProxyForwardedHeaders`, `addTailscaleIdentityHeaders`, `dialServeUnixSocket`): <https://github.com/tailscale/tailscale/blob/9d5313662a594ad9b595483153301941ae75e808/ipn/ipnlocal/serve.go#L671-L1190>
- [T6] `cmd/tailscale/cli/serve_v2.go` (help text, root requirement, availability messages): <https://github.com/tailscale/tailscale/blob/9d5313662a594ad9b595483153301941ae75e808/cmd/tailscale/cli/serve_v2.go#L139-L160>
- [T7] `ipn/ipnstate/ipnstate.go` (`DNSName`): <https://github.com/tailscale/tailscale/blob/9d5313662a594ad9b595483153301941ae75e808/ipn/ipnstate/ipnstate.go#L244-L246>
- [T8] `ipn/serve.go` (`ExpandProxyTargetValue`): <https://github.com/tailscale/tailscale/blob/9d5313662a594ad9b595483153301941ae75e808/ipn/serve.go#L764-L790>
- [T9] `ipn/ipnlocal/cert.go` (`serveTLSNextProtos`): <https://github.com/tailscale/tailscale/blob/9d5313662a594ad9b595483153301941ae75e808/ipn/ipnlocal/cert.go#L165-L167>
- [T10] `ipn/ipnlocal/serve_unix_linux.go` and `serve_unix_other.go`: <https://github.com/tailscale/tailscale/blob/9d5313662a594ad9b595483153301941ae75e808/ipn/ipnlocal/serve_unix_linux.go>

Go:

- [G1] `net/http/httputil/reverseproxy.go` (upgrade handling, removal of forwarded headers before `Rewrite`): <https://github.com/golang/go/blob/840c8327132051c7ef099b069c5b0861638170b5/src/net/http/httputil/reverseproxy.go#L105-L131>

Microsoft:

- [D1] "Dev tunnels security": <https://learn.microsoft.com/en-us/azure/developer/dev-tunnels/security>
- [D2] "Dev tunnels command-line reference": <https://learn.microsoft.com/en-us/azure/developer/dev-tunnels/cli-commands>
- [D3] "What are dev tunnels?": <https://learn.microsoft.com/en-us/azure/developer/dev-tunnels/overview>
- [D4] "Azure subscription and service limits", section "Dev tunnels limits": <https://learn.microsoft.com/en-us/azure/azure-resource-manager/management/azure-subscription-service-limits>
- [D5] `cs/src/Contracts/TunnelOptions.cs`: <https://github.com/microsoft/dev-tunnels/blob/37956c63675e3339b42c7f852a5af5da1566c5b9/cs/src/Contracts/TunnelOptions.cs#L32-L71>
- [D6] `cs/src/Contracts/TunnelEndpoint.cs` (`PortUriFormat`): <https://github.com/microsoft/dev-tunnels/blob/37956c63675e3339b42c7f852a5af5da1566c5b9/cs/src/Contracts/TunnelEndpoint.cs#L71>

ngrok:

- [N1] "HTTP/S Agent Endpoints": <https://ngrok.com/docs/universal-gateway/http>
- [N2] "ngrok CLI API": <https://ngrok.com/docs/agent/api>
- [N3] "Free Plan Limits": <https://ngrok.com/docs/pricing-limits/free-plan-limits>
- [N4] "Using ngrok with Virtual Hosts": <https://ngrok.com/docs/using-ngrok-with/virtualHosts>
- [N5] "Agent Config Version 3": <https://ngrok.com/docs/gateway/agent/config/v3>
- [N6] `upstream.go` (`WithUpstream`, `WithUpstreamDialer`): <https://github.com/ngrok/ngrok-go/blob/90313544328832331e3ba16966540973203812a7/upstream.go#L37-L87>
- [N7] "ERR_NGROK_6024": <https://ngrok.com/docs/errors/err_ngrok_6024>

Cloudflare:

- [C1] "WebSockets": <https://developers.cloudflare.com/network/websockets/>
- [C2] "Configuration file": <https://developers.cloudflare.com/tunnel/features/locally-managed-tunnels/configuration-file/>
- [C3] "Quick Tunnels": <https://developers.cloudflare.com/tunnel/get-started/quick-tunnels/>
- [C4] "Origin parameters": <https://developers.cloudflare.com/tunnel/reference/origin-parameters/>
- [C5] `ingress/origin_proxy.go` (`httpService.RoundTrip`): <https://github.com/cloudflare/cloudflared/blob/b0297f0b24c60a4cef743b7b878e2da3798cb839/ingress/origin_proxy.go#L35-L59>
- [C6] `proxy/proxy.go` (`proxyHTTPRequest`): <https://github.com/cloudflare/cloudflared/blob/b0297f0b24c60a4cef743b7b878e2da3798cb839/proxy/proxy.go#L235-L256>
- [C7] "Cloudflare HTTP headers": <https://developers.cloudflare.com/fundamentals/reference/http-headers/>
- [C8] `cmd/cloudflared/tunnel/quick_tunnel.go`: <https://github.com/cloudflare/cloudflared/blob/b0297f0b24c60a4cef743b7b878e2da3798cb839/cmd/cloudflared/tunnel/quick_tunnel.go#L115-L230>
- [C9] `ingress/ingress.go` (`unix:`, `unix+tls:`, `--unix-socket`): <https://github.com/cloudflare/cloudflared/blob/b0297f0b24c60a4cef743b7b878e2da3798cb839/ingress/ingress.go#L297-L372>
- [C10] "Connection limits": <https://developers.cloudflare.com/fundamentals/reference/connection-limits/>

Earlier research:

- [TLS] "TLS options for reaching the web monitor from another device": <https://github.com/jackgeek/climon2/blob/research/web-monitor-tls/docs/research/web-monitor-tls.md>
- [BRW] "What browsers enforce for a private-origin terminal page": <https://github.com/jackgeek/climon2/blob/research/web-monitor-browser/docs/research/web-monitor-browser.md>
