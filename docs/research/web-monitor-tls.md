# TLS options for reaching the web monitor from another device

Research for [Establish TLS options for reaching the web monitor from another device](https://github.com/jackgeek/climon2/issues/232),
a child of [Chart the web monitor specification](https://github.com/jackgeek/climon2/issues/229).

Researched 2026-11-14. This file reports facts and decides no policy. It
reuses, and does not repeat, the transport facts in
[monitor-link-transports.md](https://github.com/jackgeek/climon2/blob/research/monitor-link-transports/docs/research/monitor-link-transports.md)
(cited below as [MLT]).

Every claim cites a primary source from the list at the end. **Inference**
marks a conclusion that follows from a cited statement but is not stated by the
source. **Unverified** marks something no primary source confirmed; those are
collected in the last section.

Setting used throughout: the *web monitor* runs on the serving machine S; the
*web view* runs in a browser on a phone P. "Page script" means the web view's
HTML and script as delivered to P.

## Summary

1. **A browser cannot pin a certificate or trust it on first use for a page
   load.** Chrome removed site-set key pinning [C3], and the only inputs to a
   page load's certificate decision are the platform and user trust stores
   [C1][A1]. The single script-level pin on the web platform is WebTransport's
   `serverCertificateHashes`, which pins a connection made *by* a script that
   has already been delivered some other way, so it does not protect the page
   script itself [W2].
2. **No public certificate authority will issue for S's LAN name or private
   address.** The Baseline Requirements prohibit certificates for internal
   names and reserved IP addresses [B1]. A publicly trusted certificate
   therefore needs a public DNS name that someone controls: Tailscale's
   (`ts.net`), Microsoft's (`devtunnels.ms`) or the user's own.
3. **Four options keep the TLS private key on S**, so no third party
   terminates TLS: a private CA or self-signed certificate, a Tailscale HTTPS
   certificate (with or without `tailscale serve`), Tailscale Funnel, and ACME
   DNS-01 on a user-owned domain. In the three public-CA options, whoever
   controls the DNS zone can obtain a second valid certificate for the same
   name (**Inference** from [R1]); such a certificate would appear in public
   Certificate Transparency logs [L3][T1].
4. **A dev tunnel terminates TLS at Microsoft's service**, which then rewrites
   headers [D2]. Microsoft's service is therefore in a position to read and
   alter the page script (**Inference**).
5. **An SSH local forward needs no TLS at all**: `http://localhost` is a
   potentially trustworthy origin in the Secure Contexts specification [W1]
   and a secure origin in Chrome [C2]. WebKit's handling of loopback is
   documented by its own bug tracker as inconsistent [K1][K2]. The phone must
   run an SSH client.
6. **A private CA is the only option with no third party at all, and the one
   with the heaviest phone ceremony**: on iOS, install a profile and then
   separately enable full trust [A1]. A CA trusted this way is trusted for
   every site the phone visits, and Chrome skips pin validation for private
   trust anchors [C2], so the CA key becomes a key to all of the phone's
   browsing (**Inference**).
7. **Rust support is not the constraint.** rustls serves TLS 1.2 and 1.3
   [RS1], rcgen issues self-signed and CA-signed certificates [RS2], and
   instant-acme implements RFC 8555 including DNS-01, profiles and IP
   identifiers [RS3][RS4]. The missing piece for DNS-01 is writing the TXT
   record at the user's DNS provider, which no cited crate does.

## Comparison

| Option | Who terminates TLS | Who else could read or alter the page script | On the phone | On the serving machine | Rust support |
|---|---|---|---|---|---|
| Self-signed, click through | Web monitor on S | Anyone on the path at the moment the user accepts the warning, since nothing ties the accepted certificate to S (**Inference**) | Accept a browser warning; behaviour and persistence **Unverified** | Nothing beyond running the web monitor | rustls + rcgen [RS1][RS2] |
| Private CA installed on the phone | Web monitor on S | Nobody on the path. The CA key holder can impersonate any site to that phone (**Inference** from [C2]) | iOS: install profile, then enable full trust in Certificate Trust Settings [A1]. Android: **Unverified** | Generate CA and leaf; move the CA certificate to the phone; reissue leaves within 825 days [A2] | rustls + rcgen [RS1][RS2] |
| Tailscale HTTPS certificate (`tailscale cert`, web monitor terminates) | Web monitor on S | Not Tailscale's relays [MLT]. Tailscale controls `ts.net` DNS and could obtain its own certificate (**Inference** from [T1][R1]) | Install Tailscale, join the tailnet | Enable MagicDNS and HTTPS in the admin console; run `tailscale cert`; renew every 90 days by hand [T1] | rustls loads the files [RS1] |
| `tailscale serve` | `tailscaled` on S [T3] | As above, plus any local process that can reach the loopback backend (**Inference** from [T2]) | Install Tailscale, join the tailnet | Enable HTTPS; run `tailscale serve <port>` [T2] | None needed; the web monitor serves plain HTTP on loopback |
| Tailscale Funnel | `tailscaled` on S [T4] | Funnel relays cannot decrypt [T4]. DNS point as above | Nothing; the URL is public [T4] | Enable MagicDNS, HTTPS and the `funnel` node attribute [T4] | None needed |
| Microsoft dev tunnel | Microsoft's service ingress [D2] | Microsoft's service (**Inference** from [D2]) | Sign in with the account that created the tunnel; pass an interstitial once [D2] | Install `devtunnel`, sign in, host the port [MLT] | None needed |
| SSH local forward to loopback | Nobody; there is no TLS. `ssh` and `sshd` carry the bytes [MLT] | The `ssh` and `sshd` processes (**Inference**, [MLT]); any local process on either loopback [MLT] | Run an SSH client with local forwarding; open `http://localhost:<port>` | Run `sshd` reachable from the phone | None needed |
| ACME DNS-01 on a user-owned domain | Web monitor on S | The DNS provider and anyone holding its API credentials could obtain their own certificate (**Inference** from [R1]) | Nothing, if the name resolves to S from where the phone is | Own a domain; publish an address record for S; give the web monitor (or a helper) DNS write access; renew every 90 days or less [L1][L2] | rustls + instant-acme; DNS provider API is the caller's job [RS3][RS4] |

## Background that applies to every option

### Why HTTPS at all

- The Secure Contexts specification treats an origin as potentially
  trustworthy when its scheme is `https` or `wss`, when its host is in
  `127.0.0.0/8` or `::1/128`, or when its host is `localhost` or ends in
  `.localhost` in a user agent that follows the localhost name-resolution
  rules [W1]. A phone reaching S by LAN address over plain HTTP matches none
  of these.
- The same specification lets a user agent allow users to configure specific
  origins as trustworthy, intended for developers [W1]. Whether any phone
  browser exposes that is **Unverified**.
- The specification's algorithm decides on scheme alone and says nothing
  about certificate errors the user clicked through [W1].

### What a browser will accept as proof of the server's identity

- Chrome's certificate verifier honours local trust decisions for adding and
  removing trust on TCP connections, and does not rely on the platform's
  default root list. Its extra rules (Certificate Transparency, maximum
  validity) are stated for roots in the Chrome Root Store [C1]. The FAQ
  documents the Windows and macOS local stores; Android user-added CAs are not
  described [C1].
- Chrome on iOS cannot use the Chrome Certificate Verifier or Chrome Root
  Store because of Apple policy [C1]. **Inference**: on iOS every browser
  follows Apple's trust rules.
- "Chrome does not perform pin validation when the certificate chain chains up
  to a private trust anchor" [C2].
- Chrome proposed deprecating and removing HTTP public key pinning, the
  mechanism by which a site could tell the browser which keys to expect,
  targeting Chrome 67 and later Chrome 69 [C3]. No replacement page-level
  pinning or trust-on-first-use mechanism appears in any source read here.
- Chrome shows a non-bypassable certificate error only where the real server
  previously asked for that treatment [C2]. **Inference**: other certificate
  errors can be bypassed by the user.

### WebTransport certificate hashes: the one script-level pin

- `new WebTransport(url, { serverCertificateHashes })` makes the browser
  authenticate the server certificate against the supplied hashes "instead of
  using the Web public key infrastructure" [W2]. MDN describes the purpose as
  reaching servers that cannot easily get a public certificate, such as hosts
  that are not publicly routable [W2].
- The certificate must be X.509v3 with a validity period under two weeks,
  using ECDSA P-256 at minimum and never RSA [W2].
- WebTransport is available only in secure contexts in some or all supporting
  browsers, and MDN marks it as working across the latest browsers since March
  2026, with parts having varying support [W2].
- **Inference**: the hash is supplied by the page script, so this pins the
  data connection only. Whoever delivers the page script chooses the hash. It
  moves the problem to "where does the page come from" and does not remove it.
  It also runs over HTTP/3, not the planned WebSocket.

### What public certificate authorities will not do

- The Baseline Requirements prohibit issuing certificates containing a
  reserved IP address or internal name, effective 2015-11-01 [B1].
- Ballot SC-081v3 passed with 25 issuer votes and 4 consumer votes in favour
  and none against. It reduces maximum validity from 398 days to 47 days in
  steps from March 2026 to March 2029 [B2]. Let's Encrypt states the steps as
  200 days now, 100 days from 2027-03-15 and 47 days from 2029-03-15 [L2].

## 1. Self-signed or private-CA certificate

### Who terminates TLS and who can touch the page script

- The web monitor terminates TLS with a key that never leaves S.
- With a private CA installed on the phone, the browser validates the chain to
  that CA [C1][A1]. Nobody on the path can substitute the page script without
  the CA key or the leaf key (**Inference**).
- With a bare self-signed certificate and a click-through, the browser has no
  basis to tell S's certificate from an attacker's. The user can compare
  fingerprints by hand; whether phone browsers show a fingerprint on the
  warning page is **Unverified**.
- A CA the user enables for full trust on iOS is trusted for TLS generally
  [A1], and Chrome skips pin validation under private trust anchors [C2].
  **Inference**: anyone who obtains the CA private key can impersonate any
  site to that phone, so the CA key needs at least the protection of the
  credential it guards. X.509 name constraints could narrow this; rcgen can
  write them [RS2], but whether iOS and Android enforce them on user-added
  roots is **Unverified**.

### What the user must do

- iOS and iPadOS: a certificate profile installed by hand, from email or a
  website, "isn't automatically trusted for SSL". The user must go to
  Settings > General > About > Certificate Trust Settings and turn on full
  trust for the root. Profiles installed by Apple Configurator or MDM are
  trusted automatically [A1].
- iOS 13 and later reject TLS server certificates unless: RSA keys are at
  least 2048 bits, the signature uses SHA-2, the DNS name is in the Subject
  Alternative Name, and, for certificates issued after 2019-07-01, the
  ExtendedKeyUsage contains `id-kp-serverAuth` and the validity period is 825
  days or fewer. Violations make the connection fail [A2]. The page does not
  say whether user-added CAs are exempt [A2].
- Android: apps targeting API level 24 or above do not trust user-added CAs
  unless they opt in through their network security configuration [G1].
  Whether Chrome for Android and Firefox for Android opt in is **Unverified**.
- Serving machine: generate the CA and leaf, deliver the CA certificate to the
  phone over some channel, and reissue the leaf before it expires. The leaf
  must name whatever the phone types (a LAN name or address), which changes
  when S moves networks (**Inference**).

### Rust support

- rustls implements TLS 1.2 and 1.3 for servers, uses aws-lc-rs by default
  with *ring* as a built-in alternative, and is at 0.23.46 (2026-10-14) [RS1].
- rcgen 0.14.10 generates self-signed certificates, certificates signed by an
  issuer, and CA certificates, with ECDSA P-256/P-384, Ed25519 and RSA keys,
  and exposes extended key usage and name constraints [RS2]. That covers every
  Apple requirement above (**Inference**).

## 2. Tailscale HTTPS certificates and `tailscale serve`

### Who terminates TLS and who can touch the page script

- Tailscale requests a certificate from Let's Encrypt and completes DNS-01 by
  creating a TXT record under `ts.net`. The certificate private key and the
  ACME account key are created and stored on the user's machine: "Tailscale
  never sees them" [T1].
- With `tailscale cert`, the certificate and key are delivered as files [T1].
  **Inference**: the web monitor can load them and terminate TLS itself.
- With `tailscale serve`, "the device's Tailscale daemon terminates the HTTPS
  connection" and forwards requests to a local HTTP server such as
  `http://127.0.0.1:3000` [T3][T2]. The page script crosses loopback in clear
  between `tailscaled` and the web monitor (**Inference**).
- Serve adds `Tailscale-User-Login`, `Tailscale-User-Name` and
  `Tailscale-User-Profile-Pic` headers and strips them from incoming requests.
  Tailscale warns that anything able to call the backend directly can forge
  them, and advises listening only on localhost, which narrows forgery to
  other services on the same device [T2]. **Inference**: that remainder is
  exactly "other OS users on the same machine" in the map's threat model, so
  these headers cannot be the credential.
- Tailscale's relays cannot read tailnet traffic, and a compromised control
  plane can unless Tailnet Lock is on [MLT].
- **Inference** from [T1] and [R1]: Tailscale operates the `ts.net` zone and
  writes the DNS-01 record, so Tailscale is able to obtain a certificate for
  the machine's name with a key of its own. Let's Encrypt logs every
  certificate it issues [L3], so that would be publicly visible.

### What the user must do

- Admin console, once: enable MagicDNS, enable HTTPS, and acknowledge that
  machine names and the tailnet name are published in a public ledger.
  Tailscale warns not to enable HTTPS if machine names are sensitive [T1].
- `tailscale cert` route: run it on S; certificates last 90 days and "you are
  responsible for renewing the certificate" when it is delivered as files [T1].
  Repeated requests can hit Let's Encrypt rate limits, with waits of up to 34
  hours [T1].
- `tailscale serve` route: run `tailscale serve <port>`; the certificate is
  provisioned automatically [T3]. Serve requires HTTPS to be enabled and
  follows the tailnet's access rules [T2]. Forwarding a port does not need
  root [T2].
- Phone: must be a device in the tailnet, or one the device was shared with
  [T2]. The URL is `https://<machine>.<tailnet>.ts.net` [T2]. Bare hostnames
  get no HTTPS [T1].
- Reach: the default tailnet policy allows every device in the tailnet [MLT],
  so the credential still has to stand on its own.

### Rust support

- `tailscale cert` output is ordinary certificate and key files, which rustls
  serves [RS1] (**Inference**). `tailscale serve` needs no TLS in the web
  monitor. Obtaining certificates from `tailscaled` programmatically from Rust
  is **Unverified** and falls under the map's out-of-scope built-in Tailscale
  integration.

## 3. Tailscale Funnel

- Public DNS for the Funnel URL resolves to a Funnel relay, which opens a TCP
  proxy to the device over Tailscale. The Tailscale server on the device
  "terminates the TLS connection and passes the decrypted request to the local
  service", and "The Funnel relay server cannot decrypt data sent over this
  proxy" [T4].
- Requirements: MagicDNS, HTTPS, and a `funnel` node attribute in the tailnet
  policy. Only ports 443, 8443 and 10000, only `ts.net` names, with
  non-configurable bandwidth limits [T4].
- The phone needs nothing installed; the URL works for people who do not use
  Tailscale [T4]. **Inference**: the web monitor is then reachable by anyone
  on the internet who has the name, and the name is in public Certificate
  Transparency logs [T1], so only the credential stands between the internet
  and a shell.
- Same DNS observation and same loopback hop as `tailscale serve`.

## 4. Dev tunnels and other TLS-terminating tunnels

- For web forwarding, "TLS termination is done at service ingress using
  service certificates, issued by a Microsoft CA", and header rewriting follows
  [D2]. **Inference**: Microsoft's service holds the plaintext of the page
  script and of everything the web view sends, and is in a position to alter
  either. This is the known hard problem in its plainest form.
- The SDK's end-to-end encryption is optional and applies to tunnel
  connections, not to the browser-facing web-forwarding URL [MLT].
- Default access is private: only the account that created the tunnel. The
  phone browser must sign in with that Microsoft, Entra ID or GitHub account
  [D2]. First visits show an anti-phishing interstitial, skipped for non-GET
  requests and requests not accepting `text/html` [D2].
- The service upgrades insecure connections to HTTPS/WSS, sets HSTS for one
  year, and supports TLS 1.2 minimum [D2].
- Limits, lifetimes and the `--allow-anonymous` opening are in [MLT].
- Other vendors' TLS-terminating tunnels were not researched; see Unverified.

## 5. SSH local forward to loopback

- The phone's SSH client listens on the phone's loopback and carries
  connections to S, where `sshd` connects to the web monitor's port [MLT]. The
  browser opens `http://localhost:<port>`.
- No certificate exists in this option. Confidentiality and server identity
  come from SSH, including its host key check, between the two SSH processes
  [MLT].
- Secure context: the specification returns "Potentially Trustworthy" for
  hosts in `127.0.0.0/8` and `::1/128`, and for `localhost` where the user
  agent follows the localhost resolution rules [W1]. Chrome lists
  `(*, localhost, *)` and `(*, 127/8, *)` as secure origins [C2].
- WebKit: an open bug reports that `SecurityOrigin` treats localhost as
  potentially trustworthy while the mixed content checker and, until a commit
  of 2026-04-03, the cookie jar did not; the reporter says the inconsistency
  remains [K1]. A second bug, still open, covers loopback being blocked as
  mixed content when requested *from* an HTTPS page [K2]. A page loaded
  directly from `http://localhost` is the first case, not the second
  (**Inference**). Which shipping Safari versions behave how is **Unverified**.
- Who can read or alter the page script: the `ssh` and `sshd` processes
  (**Inference**, [MLT]), and any local process on S that can connect to the
  web monitor's loopback port or on P that can connect to the forwarded port,
  since loopback restricts by address and not by user [MLT].
- Phone: needs an SSH client app that supports local port forwarding and keeps
  running while the browser is in front. Availability and background behaviour
  on iOS and Android are **Unverified**.
- Serving machine: `sshd` must be reachable from the phone, which on a
  different network means a public address, a VPN or a jump host
  (**Inference**).
- Rust: nothing to implement; the web monitor serves plain HTTP on loopback.

## 6. ACME DNS-01 on a user-owned domain

### Who terminates TLS and who can touch the page script

- The web monitor terminates TLS with a key generated on S (**Inference**: the
  ACME client creates the key and sends only a signing request).
- DNS-01: the client publishes a TXT record at `_acme-challenge.<name>`
  holding the base64url SHA-256 digest of the key authorization, and the
  server verifies it [R1]. Let's Encrypt says this challenge can "validate
  domain names whose webservers aren't exposed to the public internet" [L1].
  That is what lets a certificate be issued for a name that resolves to a
  private address (**Inference**).
- "An entity that can provision false DNS records for a domain can attack the
  DNS challenge directly" [R1]. **Inference**: the DNS provider, and anyone
  holding the DNS API credentials, can get their own certificate for the name.
- Let's Encrypt warns that "Keeping API credentials on your web server is
  risky" and recommends narrowly scoped credentials or validating from a
  separate machine [L1]. For climon2 the "web server" is the user's own
  workstation.
- Every certificate Let's Encrypt issues goes to Certificate Transparency
  logs [L3], so the chosen name becomes public. A wildcard certificate, which
  DNS-01 allows [L1], avoids publishing the machine's label (**Inference**).

### What the user must do

- Own a domain at a DNS provider with an API [L1]; publish an address record
  pointing at S's LAN or VPN address; arrange renewal.
- Lifetimes at Let's Encrypt: `classic` 90 days, `tlsserver` 45 days,
  `shortlived` 160 hours [L2]. Short-lived certificates are opt-in by
  selecting the profile [L4]. Authorization reuse is 30 days for `classic` and
  7 hours for the other two [L2], so short profiles need DNS write access at
  nearly every renewal (**Inference**).
- DNS-PERSIST-01, announced 2026-02-18, replaces the per-issuance token with a
  standing TXT record at `_validation-persist.<name>` naming the CA and the
  ACME account, after which "protecting the ACME account key becomes the
  central concern". Production was targeted for Q2 2026 [L5]. Whether it is
  live is **Unverified**. If it is, DNS credentials would be needed once, at
  setup, and never on S afterwards (**Inference**).
- Let's Encrypt issues IP address certificates only as short-lived
  certificates [L4], DNS-01 "cannot be used to validate IP Addresses" [L1],
  and reserved addresses are barred anyway [B1]. A certificate for S's private
  address is not available by any route (**Inference**).
- The alternatives to DNS-01 need inbound reachability: HTTP-01 goes to TCP
  port 80 [R1][L1] and TLS-ALPN-01 to port 443 [L1].
- Phone: nothing to install or trust. The name must resolve to an address the
  phone can reach.

### Rust support

- instant-acme 0.8.5 (2026-02-24) is an async RFC 8555 client with ARI,
  profiles, external account binding and revocation, using hyper, rustls and
  Tokio, with aws-lc-rs or *ring*; only P-256 account keys [RS3]. Its
  `ChallengeType` has `Dns01`, `Http01` and `TlsAlpn01`; its `Identifier` has
  `Dns` and `Ip`; `NewOrder` has a `profile` setter [RS4].
- instant-acme does not write DNS records; the caller must publish the TXT
  value (**Inference** from [RS3], which lists no DNS provider support). A
  Rust library covering DNS providers' APIs is **Unverified**.
- rustls-acme 0.15.4 implements TLS-ALPN-01 (default) and HTTP-01 only, with
  no mention of DNS-01 [RS5], so it does not fit a machine with no inbound
  internet reachability (**Inference**).

## Rust TLS stack advisories

Read from the GitLab Advisory Database's mirror of GitHub advisories, not from
the projects' own pages, which the fetcher could not reach. Affected and fixed
versions were not shown.

- rustls [V1]: 2024, a denial of service (CVE-2024-32650) and a
  network-reachable panic in `Acceptor::accept` (GHSA-qg5g-gv98-5ffh). 2026,
  five advisories: `Debug` formatting exposing session secrets
  (GHSA-c8m5-c952-6cm4), ECH rejection exposing buffered data
  (GHSA-vp88-798v-h67f), Brotli certificate decompression allocating up to
  1 GiB before authentication (GHSA-h2v8-394m-p7vv), a ServerHello downgrading
  later key shares to classical X25519 (GHSA-w44r-93j7-qv28), and TLS 1.3
  handshake messages accepted across encryption levels (GHSA-2mjx-qc3c-rqvc).
- rustls-webpki [V2]: 2023, CPU denial of service in path building
  (GHSA-fh2r-99q2-6mmg). 2026, a panic on a malformed CRL
  (GHSA-82j2-j2ch-gfr8), two name-constraint acceptance bugs
  (GHSA-xgp8-3hg3-c2mh, GHSA-965h-392x-2mh5) and a CRL distribution-point
  matching bug (GHSA-pwjx-qhcg-rvj4).
- **Inference**: two of these are reachable by an unauthenticated peer of a
  TLS server (the `Acceptor::accept` panic and the cross-level handshake
  messages), which is the web monitor's position on any option where it
  terminates TLS itself. Options where `tailscaled`, `sshd` or a tunnel
  terminates move that exposure off climon2's code.

## Unverified

- Whether a clicked-through certificate error yields a secure context in
  Chrome, Safari or Firefox on phones, whether the exception persists, and
  whether the warning page shows a fingerprint the user could compare.
- Whether Chrome for Android and Firefox for Android trust user-added CAs.
  The Chrome Root Store FAQ documents only Windows and macOS local stores
  [C1]; no Mozilla primary source was reached.
- Whether iOS or Android enforce X.509 name constraints on user-added roots.
- Whether Apple's iOS 13 certificate requirements, or its later 398-day limit,
  apply to certificates under user-added roots. [A2] does not say; Apple's
  398-day page could not be fetched.
- Whether any phone browser lets the user mark an HTTP origin as trustworthy.
- Per-browser support for `serverCertificateHashes`, especially Safari on iOS.
  The W3C WebTransport text could not be fetched; the facts above are MDN's.
- How shipping Safari versions treat `http://localhost` as a secure context.
- Availability of SSH clients with local forwarding on iOS and Android, and
  whether a forward survives the app being backgrounded.
- Whether DNS-PERSIST-01 is live in Let's Encrypt production.
- Which challenge types Let's Encrypt accepts for IP identifiers.
- Fixed versions for the rustls and rustls-webpki advisories; whether rcgen,
  instant-acme or tokio-rustls have advisories.
- Any Rust crate that writes DNS-01 records at DNS providers.
- Fetching certificates from `tailscaled` programmatically from Rust.
- Whether `tailscale serve` passes WebSocket upgrades. No page read says so.
- Cloudflare Tunnel, ngrok and other TLS-terminating tunnels: not researched.
- The current Baseline Requirements text. Version 2.1.4's relevant-dates table
  was read [B1]; later versions were not.

## Sources

Web platform:

- [W1] W3C, "Secure Contexts" (editor's draft), sections 3.1, 5.2, 7.2: <https://w3c.github.io/webappsec-secure-contexts/>
- [W2] MDN, "WebTransport: WebTransport() constructor": <https://developer.mozilla.org/en-US/docs/Web/API/WebTransport/WebTransport>

Chromium:

- [C1] "Chrome Root Store: Frequently Asked Questions": <https://chromium.googlesource.com/chromium/src/+/main/net/data/ssl/chrome_root_store/faq.md>
- [C2] "Chrome Security FAQ" at commit `dde7637f`: <https://chromium.googlesource.com/chromium/src/+/dde7637fb2fe94de4c02dbe684070a83463e0708/docs/security/faq.md>
- [C3] blink-dev, "Intent To Deprecate And Remove: Public Key Pinning": <https://groups.google.com/a/chromium.org/g/blink-dev/c/he9tr7p3rZ8/m/eNMwKPmUBAAJ>

Apple and WebKit:

- [A1] "Trust manually installed certificate profiles in iOS, iPadOS, and visionOS": <https://support.apple.com/102390>
- [A2] "Requirements for trusted certificates in iOS 13 and macOS 10.15": <https://support.apple.com/103769>
- [K1] WebKit bug 281149, "WebKit is inconsistent about whether localhost is a secure origin or not": <https://bugs.webkit.org/show_bug.cgi?id=281149>
- [K2] WebKit bug 171934, "Don't treat loopback addresses ... as mixed content": <https://bugs.webkit.org/show_bug.cgi?id=171934>

Android:

- [G1] Android Developers Blog, "Changes to Trusted Certificate Authorities in Android Nougat": <https://android-developers.googleblog.com/2016/07/changes-to-trusted-certificate.html>

CA/Browser Forum:

- [B1] TLS Baseline Requirements 2.1.4, sections 1.2.2 and 1.6.1: <https://cabforum.org/working-groups/server/baseline-requirements/documents/CA-Browser-Forum-TLS-BR-2.1.4.pdf>
- [B2] "Ballot SC-081v3: Introduce Schedule of Reducing Validity and Data Reuse Periods": <https://cabforum.org/2025/04/11/ballot-sc081v3-introduce-schedule-of-reducing-validity-and-data-reuse-periods/>

ACME and Let's Encrypt:

- [R1] RFC 8555, sections 8.3, 8.4, 10.2, 11.2: <https://www.rfc-editor.org/rfc/rfc8555>
- [L1] "Challenge Types" (updated 2026-02-12): <https://letsencrypt.org/docs/challenge-types/>
- [L2] "Profiles": <https://letsencrypt.org/docs/profiles/>
- [L3] "Certificate Transparency (CT) Logs": <https://letsencrypt.org/docs/ct-logs/>
- [L4] "6-day and IP Address Certificates are Generally Available" (2026-01-15): <https://letsencrypt.org/2026/01/15/6day-and-ip-general-availability.html>
- [L5] "DNS-PERSIST-01" (2026-02-18): <https://letsencrypt.org/2026/02/18/dns-persist-01.html>

Tailscale:

- [T1] "Enabling HTTPS": <https://tailscale.com/docs/how-to/set-up-https-certificates>
- [T2] "Tailscale Serve": <https://tailscale.com/docs/features/tailscale-serve>
- [T3] "tailscale serve command": <https://tailscale.com/docs/reference/tailscale-cli/serve>
- [T4] "Tailscale Funnel": <https://tailscale.com/docs/features/tailscale-funnel>

Microsoft:

- [D2] "Dev tunnels security": <https://learn.microsoft.com/azure/developer/dev-tunnels/security>

Rust:

- [RS1] rustls 0.23.46: <https://docs.rs/crate/rustls/latest>
- [RS2] rcgen 0.14.10: <https://docs.rs/rcgen>
- [RS3] instant-acme 0.8.5 README: <https://docs.rs/crate/instant-acme/latest>
- [RS4] instant-acme 0.8.5 `src/types.rs`: <https://docs.rs/crate/instant-acme/latest/source/src/types.rs>
- [RS5] rustls-acme 0.15.4: <https://docs.rs/rustls-acme/>
- [V1] GitLab Advisory Database, rustls: <https://advisories.gitlab.com/pkg/cargo/rustls/>
- [V2] GitLab Advisory Database, rustls-webpki: <https://advisories.gitlab.com/pkg/cargo/rustls-webpki/>

Earlier climon2 research:

- [MLT] "What SSH forwards, Tailscale and dev tunnels guarantee to a TCP monitor link": <https://github.com/jackgeek/climon2/blob/research/monitor-link-transports/docs/research/monitor-link-transports.md>

## Method notes

- All pages were read through a summarising fetcher. Quoted phrases are as it
  returned them and were not checked character for character against the
  rendered pages.
- Not reachable through the fetcher, so not read: the W3C WebTransport
  specification, Apple's 398-day page, and the rustls project's own advisory
  list.
- No behaviour was tested on a device.
