# What SSH forwards, Tailscale and dev tunnels guarantee to a TCP monitor link

Research for [Establish what SSH forwards, Tailscale and dev tunnels guarantee to a TCP monitor link](https://github.com/jackgeek/climon2/issues/191),
a child of [Chart the monitor link specification](https://github.com/jackgeek/climon2/issues/189).

Researched 2026-11-14. This file reports facts and decides no policy. climon2
treats all three transports as untrusted byte pipes regardless of what is
written here.

Every claim cites a primary source from the list at the end. Claims marked
**Unverified** could not be confirmed from a primary source; claims marked
**Inference** follow from a cited statement but are not themselves stated by
the source.

Setting used throughout: the *relay* listens on a TCP port on machine R; the
*hub monitor* on machine H dials it.

## Summary

| | SSH port forward (OpenSSH) | Tailscale | Microsoft dev tunnel |
|---|---|---|---|
| Carries raw TCP | Yes | Yes (IP-level overlay) | `devtunnel connect` forwards a local TCP port over SSH port forwarding; the public URL is HTTP(S)/WS(S) only |
| Default reachability of the forwarded port | Loopback of the machine holding the listener | Every device in the tailnet (default policy allows all) | Only the account that created the tunnel |
| Widest documented opening | Wildcard bind (`GatewayPorts`, `-g`, `*:`) | Funnel: anyone on the internet, TLS only, ports 443/8443/10000 | `--allow-anonymous`: anyone on the internet who has the tunnel ID |
| Who can read or alter the bytes | The `ssh` and `sshd` processes at each end of the secure channel (Inference); not the network between them | Not DERP relays or Funnel relays; a compromised control plane can, unless Tailnet Lock is on | Microsoft's service terminates TLS for web forwarding; SDK end-to-end encryption is optional and negotiated |
| Address the hub dials | `localhost:port` (or chosen bind address) | `100.x.y.z:port` or MagicDNS name | `localhost:port` after `devtunnel connect`; or `https://<id>-<port>.devtunnels.ms` |
| Idle behaviour | No idle timeout and no keepalive probes by default | **Unverified** | **Unverified** for connections; tunnels deleted after 30 days of inactivity |
| Hard limits | None documented | "Limited quality of service" on DERP; Funnel bandwidth limits | 5 GB/user/month, 20 MB/s per tunnel, 1000 connections per port |
| macOS / Linux / Windows | Windows documented by Microsoft; macOS and Linux **Unverified** from a primary source | Install pages exist for all three; minimum versions **Unverified** | All three |

## 1. SSH port forward (OpenSSH)

Sources: OpenSSH manual pages `ssh.1`, `ssh_config.5`, `sshd_config.5` at
openssh-portable commit `268bdcb0` [S1][S2][S3]; Microsoft's OpenSSH for Windows
overview [S4].

Two shapes are relevant:

- **Local forward** (`ssh -L`, run on H): `ssh` on H listens; connections are
  carried to R's side and `sshd` on R connects to the relay's port.
- **Remote forward** (`ssh -R`, run on R): `sshd` on the SSH server listens;
  connections are carried back and `ssh` on R connects to the relay's port.

### Who can reach the forwarded port

- Local forward: by default `ssh` binds local port forwardings to the loopback
  address, which "prevents other remote hosts from connecting to forwarded
  ports". `GatewayPorts yes` (client option, default `no`) binds them to the
  wildcard address instead [S2, `GatewayPorts`]. `-g` "Allows remote hosts to
  connect to local forwarded ports" [S1, `-g`]. An explicit `bind_address` in
  `-L` overrides the default [S1, `-L`].
- Remote forward: "By default, TCP listening sockets on the server will be bound
  to the loopback interface only." An empty `bind_address` or `*` asks for all
  interfaces, but "Specifying a remote bind_address will only succeed if the
  server's GatewayPorts option is enabled" [S1, `-R`]. The server option
  `GatewayPorts` takes `no` (default; local host only), `yes` (force wildcard)
  or `clientspecified` [S3, `GatewayPorts`].
- Loopback binding restricts by network address, not by user. The manual pages
  say nothing about restricting which local users may connect to a forwarded
  TCP port. **Inference**: any local process on the machine holding the
  listener can connect.
- Both `-L` and `-R` also accept a Unix socket in place of either TCP end
  [S1, `-L`, `-R`].
- Privileged ports: "Only the superuser can forward privileged ports" (local);
  "Privileged ports can be forwarded only when logging in as root on the remote
  machine" (remote) [S1].
- Server-side gates: `AllowTcpForwarding` (`yes` default, `no`, `local`,
  `remote`) [S3]; `PermitOpen` limits forward destinations and `PermitListen`
  limits remote-forward listen addresses, both permitting everything by
  default [S3]. The manual notes "disabling TCP forwarding does not improve
  security unless users are also denied shell access, as they can always
  install their own forwarders" [S3, `AllowTcpForwarding`].
- `-R` with port `0` makes the server allocate the listen port dynamically and
  report it to the client at run time [S1, `-R`].

### What intermediate hosts can read or alter

- The forwarded connection "is forwarded over the secure channel" between `ssh`
  and `sshd` [S1, `-L`, `-R`].
- **Inference**: the secure channel ends at the `ssh` and `sshd` processes, so
  those two processes handle the forwarded bytes in the clear, and the two
  outer hops (dialler to listener, and forwarder to relay port) are ordinary
  unencrypted TCP. The manual pages do not state this in so many words. Where
  the SSH server is a third machine (a jump or bastion host) its `sshd` is one
  of those two processes.

### How addresses and ports are presented

- Raw TCP. The hub dials `bind_address:port` of the listener, by default a
  loopback address and a port chosen by the user (or by the server with `-R 0`)
  [S1].
- The final hop to the relay is a new TCP connection "made to either host port
  hostport ... from the remote machine" (`-L`) or "from the local machine"
  (`-R`) [S1]. **Inference**: the relay therefore sees the forwarding process's
  address as the peer address, not the hub monitor's. The manual does not
  describe any way for the destination to learn the originator's address.
- The address and port the hub dials need not equal the address and port the
  relay listens on; the mapping is whatever the user wrote in `-L`/`-R` [S1].

### Lifetime, idle and keepalive

- The forward exists only while the SSH connection does. `ExitOnForwardFailure`
  (default `no`) controls whether `ssh` exits if it cannot set up a requested
  forwarding; it "does not apply to connections made over port forwardings"
  [S2].
- Application-level probes are off by default at both ends:
  `ServerAliveInterval` default `0` (client) [S2] and `ClientAliveInterval`
  default `0` (server) [S3]. When enabled, the count defaults to 3; the manual's
  example of interval 15 gives disconnection "after approximately 45 seconds"
  [S2][S3]. These messages travel "through the encrypted channel and therefore
  will not be spoofable" [S2].
- `TCPKeepAlive` defaults to `transport`: TCP keepalives on the SSH transport
  connection only. `all` extends them to "sockets created for X11 or port
  forwarding". The manual warns that with keepalives "connections may terminate
  if the connection is suffers a transient disruption", and that TCP keepalive
  "is spoofable" [S2][S3].
- No inactivity timeout by default: `ChannelTimeout` "default is not to expire
  channels of any type for inactivity" (both ends) [S2][S3], and
  `UnusedConnectionTimeout` defaults to `none`. Notably, "Forwarding listeners,
  such as those from the ssh(1) -R flag, are not considered as open channels and
  do not prevent the timeout" when it is set [S3].
- **Unverified**: behaviour of NAT devices or firewalls between `ssh` and
  `sshd` that drop idle TCP flows. Not an OpenSSH property; no primary source
  consulted.

### Platform availability

- Windows: "Beginning with Windows 10 build 1809 and Windows Server 2019,
  OpenSSH is available as a feature on demand"; default state is "Not
  installed" on Windows Server 2019/2022 and Windows 10 1809+, and "Installed
  but not enabled" on Windows Server 2025. Components include `ssh` and `sshd`
  [S4].
- **Unverified**: that macOS and mainstream Linux distributions ship OpenSSH by
  default. Widely true, but no Apple or distribution primary source was
  consulted.

## 2. Tailscale

Sources: Tailscale's official documentation [T1]–[T9].

### Who can reach the port

- A tailnet is "inaccessible from the public internet" [T1]; Tailscale IP
  addresses "aren't exposed to the public internet" [T2].
- Rules are deny-by-default once written, but "the default tailnet policy file
  enables communication between all devices within the tailnet" [T3]. So with
  an unmodified policy every device in the tailnet can reach the relay's port.
- Rules can restrict by source and by destination `host:ports` [T4], are
  directional ("Allowing a source to connect to a destination doesn't mean the
  destination can connect to the source"), and are enforced by the receiving
  device: "A device enforces incoming connections based on the access rules
  distributed to all devices in your tailnet" [T3].
- **Funnel** widens reach to "anyone ... even if they don't use Tailscale". It
  is "disabled by default", listens only on ports 443, 8443 and 10000, "only
  works over TLS-encrypted connections", and works only with names in the
  tailnet's `ts.net` domain [T5]. A plain-TCP monitor link is therefore not
  directly diallable through Funnel without a TLS client in front of it
  (**Inference** from the TLS-only statement).
- **Unverified**: whether a listener must bind the Tailscale interface address
  or the wildcard address to be reachable over the tailnet, and reachability
  via device sharing with other tailnets.

### What the operator or intermediate hosts can read or alter

- Direct, peer-relay and DERP-relayed connections are all "end-to-end encrypted
  with WireGuard" [T6].
- "DERP servers never access your unencrypted data; they only handle WireGuard
  packets that are already encrypted" [T7]; "it's impossible for a DERP server
  to decrypt your traffic" [T8].
- Control plane: its "most critical function" is "the distribution of node
  keys", and without Tailnet Lock "a compromised control plane could read or
  write traffic by distributing attacker-generated keys". With Tailnet Lock,
  "Tailscale infrastructure cannot add an unauthorized node to a tailnet" [T9].
  Tailnet Lock is enabled by an administrator action [T9]. **Unverified**:
  that it is off by default (the white paper implies but does not state it).
- Funnel: "The Funnel relay server cannot decrypt data sent over this proxy";
  the Tailscale server on the user's own device "terminates the TLS connection
  and passes the decrypted request to the local service" [T5].
- Tailscale's encryption page says its description reflects the implementation
  "as of December 2024" and is not a commitment to keep it indefinitely [T7].

### How addresses and ports are presented

- Raw IP overlay, so plain TCP to an address and port works unchanged
  (**Inference** from "Each device in your tailnet receives a private Tailscale
  IP address" [T1]).
- Each device gets a unique IPv4 address in `100.64.0.0/10` (RFC 6598 shared
  address space) which "remain[s] constant regardless of the device's physical
  location", including when it switches networks [T2].
- Each tailnet has a tailnet DNS name used for MagicDNS [T1]. **Unverified**:
  the exact form of per-device MagicDNS names, IPv6 tailnet addresses, and
  that the relay sees the hub's Tailscale address as the TCP peer address.
- The port the hub dials is the port the relay listens on; Tailscale does no
  port mapping for tailnet traffic (**Inference**; not stated).

### Lifetime, idle and keepalive

- "All connections start as relayed through a DERP server", Tailscale then
  tries to upgrade to direct, "periodically re-checks", and "an existing direct
  connection [can] fall back to a relayed connection" if conditions change
  [T6]. Path changes happen beneath the TCP connection.
- Node keys expire: "new domains are set with an expiry period of 180 days"
  (configurable 1–180 days; can be disabled per device). "If reauthentication
  does not occur, keys expire and connections to/from the given endpoint will
  stop working" [T10].
- "DERP servers are reliable but have limited quality of service" [T6]. Funnel
  traffic "is subject to non-configurable bandwidth limits" [T5].
- **Unverified**: any idle timeout or keepalive Tailscale applies to an idle
  TCP flow, and numeric DERP throughput limits. The pages consulted do not
  address idle behaviour.

### Platform availability

- Tailscale publishes separate install pages for Linux, macOS and Windows [T11].
  **Unverified**: minimum OS versions (the index page lists none).
- Funnel "only works on platforms that can run the Tailscale CLI"; on macOS it
  needs the open source variant of the app [T5].

## 3. Microsoft dev tunnels

Sources: Microsoft Learn dev tunnels documentation [D1]–[D5] and the
`microsoft/dev-tunnels` SDK source at commit `37956c63` [D6]. The `devtunnel`
CLI itself is closed source; SDK facts describe the SDK, and whether the CLI
uses the same settings is **Unverified**.

Microsoft states the service "is for adhoc testing and development, not for
production workloads", is in public preview "without a service-level
agreement" [D1], and that CLI "Command names and options may change in future
releases" [D3].

### Who can reach the port

- "By default, tunnels and tunnel ports are private and only accessible to the
  user who created the tunnel"; hosting and connecting require the same
  Microsoft, Entra ID or GitHub account [D2]. Creating a tunnel always needs an
  account [D4].
- Access can be widened to the current Entra tenant (`--tenant`) or a GitHub
  organisation (`--org`), per tunnel or per port (`--port`) [D2][D3].
- Anonymous access: `--allow-anonymous` adds an access control entry. Microsoft
  warns this "means anyone on the internet is able to connect to your local
  server, if they can guess the dev tunnel ID" [D3]. It can be time-limited
  (1 hour to 30 days) [D4].
- Bearer tokens: `devtunnel token` issues tokens that "grant limited access to
  anyone holding the token"; a client access token "allows the bearer to
  connect to any ports of the tunnel". Tokens "expire after some time
  (currently 24 hours)" [D2].
- "A dev tunnel only allows connections on ports that have been added" [D1].

### What the operator or intermediate hosts can read or alter

- All traffic goes through Microsoft's relay service, which "Facilitates secure
  connections between a dev tunnel host and clients via a cloud service" [D1].
- Web forwarding: "TLS termination is done at service ingress using service
  certificates, issued by a Microsoft CA", and "After TLS termination, header
  rewriting takes place" [D2]. The service therefore reads and modifies
  web-forwarded traffic. Tunnel traffic can also be inspected in browser
  DevTools [D1].
- SDK relay connections (source comments, [D6]):
  - The outer SSH session to the relay prefers key exchange `none`: "encryption
    of the outer SSH session is optional since it is already over a TLS
    websocket" (`cs/src/Connections/TunnelClient.cs` L189–190). For host
    sessions "there is no SSH encryption or key-exchange"
    (`TunnelRelayConnection.cs` L226).
  - Per-port end-to-end encryption between client and host is a negotiated
    option: the client requests it, and "relay or host may enable E2E
    encryption or not depending on capabilities and policies"
    (`Messages/PortRelayConnectResponseMessage.cs` L19–28). The SDK property
    `EnableE2EEncryption` defaults to `true` (`TunnelConnection.cs` L174).
  - **Inference**: when end-to-end encryption is not negotiated, the relay
    service handles the forwarded bytes in the clear inside its TLS
    terminations.
- The Learn documentation pages consulted do not mention end-to-end encryption
  at all [D1][D2].

### How addresses and ports are presented

- Public form: an HTTPS URL such as `https://tunnelid-3000.devtunnels.ms`,
  for "Tunnel ports using the HTTP(S)/WS(S) protocols" [D2]. Port protocol
  options are only `http`, `https` or `auto` [D3]. Browsers get an interstitial
  anti-phishing page on first visit, and insecure connections are upgraded to
  HTTPS/WSS [D2]. This path is HTTP-only and not usable by a plain TCP dialler.
- Client forwarding form: `devtunnel connect TUNNELID` "forward[s] connections
  from a port on the client to a dev tunnel port". The documented output is
  `SSH: Forwarding from 127.0.0.1:3000 to host port 3000` and
  `[::1]:3000`, making the service "available at localhost:3000 on the client,
  using either IPv4 or IPv6" [D3]. So the hub dials loopback on the same port
  number as the host's port; no CLI flag for changing the local port or bind
  address is documented [D3]. In the SDK the local listener address defaults to
  loopback and is settable (`TunnelClient.cs` L110–116) [D6].
- A tunnel connection is "A duplex stream of packets ... most often linked to
  TCP connections on either side, but UDP-based protocols may also be
  supported" [D1]. The SDK forwards ports with SSH `forwarded-tcpip` /
  `direct-tcpip` channels (`TunnelRelayTunnelHost.cs` L243, L636–645) [D6].
  **Unverified**: an explicit Microsoft statement that arbitrary non-HTTP TCP
  protocols are supported through `devtunnel connect`.
- Tunnel ports are numbers 1–65535 [D1]. The tunnel host "forwards those
  connections to local ports" [D1]; **Inference**: the relay sees the local
  host process as its TCP peer, not the hub.
- A tunnel ID can be chosen and reused; reusing a tunnel keeps the
  web-forwarding URL stable [D4].

### Lifetime, idle and limits

- `devtunnel host` without a prior `devtunnel create` makes "a temporary tunnel
  that is deleted once the connection is closed"; `devtunnel create` makes a
  persistent one [D4].
- Unused tunnels are deleted "after 30 days of no activity" by default;
  `--expiration` sets 1 hour to 30 days as a sliding inactivity window [D4].
- Limits, reset monthly [D5]:

  | Resource | Limit |
  |---|---|
  | Bandwidth | 5 GB per user |
  | Tunnels | 10 per user |
  | Active connections | 1000 per port |
  | Ports | 10 per tunnel |
  | HTTP request rate | 1500/min per port |
  | Data transfer rate | Up to 20 MB/s per tunnel |
  | Max web-forwarding HTTP request body size | 16 MB |

- SDK features differ by language: reconnection, SSH-level reconnection, token
  refresh and SSH keep-alive are supported in C# and TypeScript only; Rust
  supports client and host connections but none of those four [D6, README].
- **Unverified**: any idle timeout on an established tunnel connection, what
  happens to open connections when the monthly bandwidth limit is reached, and
  the CLI's own keepalive and reconnect behaviour.

### Platform availability

- "Dev tunnels are available cross-platform on Windows, Linux, and macOS" [D4].

## Sources

OpenSSH (manual page sources, openssh-portable commit
`268bdcb0c80876d97abdc74a1ca8992a1fdf7652`, 2026-11-11):

- [S1] `ssh.1`: <https://github.com/openssh/openssh-portable/blob/268bdcb0c80876d97abdc74a1ca8992a1fdf7652/ssh.1>
- [S2] `ssh_config.5`: <https://github.com/openssh/openssh-portable/blob/268bdcb0c80876d97abdc74a1ca8992a1fdf7652/ssh_config.5>
- [S3] `sshd_config.5`: <https://github.com/openssh/openssh-portable/blob/268bdcb0c80876d97abdc74a1ca8992a1fdf7652/sshd_config.5>
- [S4] Microsoft, "OpenSSH for Windows overview": <https://learn.microsoft.com/windows-server/administration/openssh/openssh_overview>

Tailscale:

- [T1] "What is a tailnet?": <https://tailscale.com/docs/concepts/tailnet>
- [T2] "What are these 100.x.y.z addresses?": <https://tailscale.com/docs/concepts/tailscale-ip-addresses>
- [T3] "Manage permissions using ACLs": <https://tailscale.com/docs/features/access-control/acls>
- [T4] "Syntax reference for the tailnet policy file": <https://tailscale.com/docs/reference/syntax/policy-file>
- [T5] "Tailscale Funnel": <https://tailscale.com/docs/features/tailscale-funnel>
- [T6] "Connection types": <https://tailscale.com/docs/reference/connection-types>
- [T7] "Tailscale encryption": <https://tailscale.com/docs/concepts/tailscale-encryption>
- [T8] "DERP servers": <https://tailscale.com/docs/reference/derp-servers>
- [T9] "Tailnet Lock white paper": <https://tailscale.com/docs/concepts/tailnet-lock-whitepaper>
- [T10] "Key expiry": <https://tailscale.com/docs/features/access-control/key-expiry>
- [T11] "Install Tailscale": <https://tailscale.com/docs/install>

Microsoft dev tunnels:

- [D1] "What are dev tunnels?": <https://learn.microsoft.com/azure/developer/dev-tunnels/overview>
- [D2] "Dev tunnels security": <https://learn.microsoft.com/azure/developer/dev-tunnels/security>
- [D3] "Dev tunnels command-line reference": <https://learn.microsoft.com/azure/developer/dev-tunnels/cli-commands>
- [D4] "Frequently asked questions": <https://learn.microsoft.com/azure/developer/dev-tunnels/faq>
- [D5] "Azure subscription and service limits", section "Dev tunnels limits": <https://learn.microsoft.com/azure/azure-resource-manager/management/azure-subscription-service-limits>
- [D6] `microsoft/dev-tunnels` SDK at commit `37956c63675e3339b42c7f852a5af5da1566c5b9` (2026-11-05): <https://github.com/microsoft/dev-tunnels/tree/37956c63675e3339b42c7f852a5af5da1566c5b9>

## Method notes

- OpenSSH manual text was read from a clone of the upstream portable
  repository, not from a rendered mirror. Deployed OpenSSH versions may predate
  options such as `ChannelTimeout` and `UnusedConnectionTimeout`.
- Tailscale and Microsoft pages were read through a summarising fetcher; quoted
  phrases are as returned by it and were not checked character-for-character
  against the rendered pages.
- No behaviour was tested empirically.
