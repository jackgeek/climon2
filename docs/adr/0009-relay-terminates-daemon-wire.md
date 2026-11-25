# The relay terminates daemon wire revision 1

A monitor link lets a hub monitor on one machine see and drive the sessions on another, through a relay. We decided that the relay is an ordinary client of its own machine's session daemons: it loads the version-matched client library for each session, as a monitor does, and the monitor link carries the client interface's records (requests one way, deliveries and the run report the other) in a link protocol of its own. Daemon wire revision 1 never crosses a machine boundary. This keeps version matching on the machine where the releases are installed, and leaves the client interface, revision 1 and the same-user trust checks unchanged. It is decided in [#195](https://github.com/jackgeek/climon2/issues/195).

## Considered Options

- **Forward revision 1 end to end.** The hub monitor would load the client library matching each remote daemon's release, and the relay would pipe bytes to the daemon's socket. Rejected. The hub would need every remote machine's releases installed for its own platform. The client library reads a metadata file and opens the socket by path, so it would have to accept a byte stream instead, reopening the agreed client interface. Revision 1 has no flow-control record and drops a connection after five seconds without write progress, which suits a local socket and not a slow link shared by many sessions.

## Consequences

- To a session daemon the relay is just another client, so the control rules hold unchanged: the most recent takeover wins, and a running relay never stops a local monitor or `c2 attach` from taking a session.
- The relay is an endpoint of the monitor link, not a pipe. It sees every relayed session's content in the clear.
- The link protocol is a second protocol with its own versioning, separate from daemon releases.
- One connection carries every session stream, so the link needs its own per-stream flow control. A hub monitor that falls behind loses that one session's client, not the link.
- The hub monitor learns a remote session's status and notifications only through that session's stream. Notifications raised while a link is down are not replayed.
