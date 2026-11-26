# One home, one host

A climon2 home belongs to one host: one machine and, on Linux, one PID namespace. A session's PID and process identity mean something only there, so a home shared with another machine (a network home directory) or another container would let one host take the other's live sessions for ended ones, or never clean up after them. We decided that climon2 refuses such a home instead of trying to keep both hosts' sessions apart, and that `c2 relay` is how you work across machines. It is decided in [#133](https://github.com/jackgeek/climon2/issues/133) (owner, 2026-11-26), and supersedes the cross-machine parts of [ADR 0003](0003-platform-and-shell-seams.md) and [ADR 0005](0005-frozen-session-metadata-core.md).

## The home owner

The home records its owner in `<climon2 home>/owner`, a JSON object holding the owner's origin, as a session's `daemon_origin` does (ADR 0005):

```json
{"machine_id":"8f2c4d1e9a7b4c3d8e6f5a4b3c2d1e0f","pid_namespace":4026531836}
```

- `machine_id` is the machine's stable identity: on Linux systemd's `/etc/machine-id`, on macOS the hardware UUID from `gethostuuid`, on Windows `MachineGuid`. On Linux it is absent when `/etc/machine-id` is missing, and when it is the transient ID systemd generates at each boot and mounts from `/run/machine-id` (the last mount at `/etc/machine-id` in `/proc/self/mountinfo` is the root `/machine-id` of a tmpfs). A container's bind mount of its host's real file is not transient.
- `pid_namespace` is Linux only: the inode of `/proc/self/ns/pid`.
- At least one member is present. Unknown members are ignored, so later releases can add to the record. A record that is not such an object, or is not a plain file, is refused and never replaced.

Two origins are the **same host** when their PID namespaces are equal (both absent on macOS and Windows) and, when both have a machine ID, so are their machine IDs. On Linux, where either side has no stable machine ID, the PID namespace alone decides. On macOS and Windows the machine ID alone decides.

## The check

`exec`, `attach`, `list`, `kill`, the monitor (through `c2 monitor`), and the session daemon when it starts, each check the home before they do anything else with it. `--help` and `--version` do not. The check lives in one place, `climon2_platform::owner::check`.

- **No owner record:** the check claims the home for this host. It writes the record to an owner-only file of its own and hard-links it into place, so the record appears whole and once. When several first commands race, the one whose record is in place owns the home, and the others are judged against it. A home with no record after an upgrade is claimed the same way, without a prompt.
- **This host is the owner:** the command goes on.
- **Another host is the owner:** the command exits 125 before any session is started, attached, listed or killed, with: "this climon2 home is in use by another machine or container (<owner origin>). To work across machines, use `c2 relay`. To move this home here, run `c2 home release` on the owning host." A daemon reports a short static form of it to the CLI that started it and logs the full refusal.
- **This host cannot read its own origin:** with no owner record, the command goes on without claiming the home, and warns. With one, it refuses, saying the origin could not be checked.
- **The record cannot be read, or is not valid:** the command refuses, and the record is left as it is.
- **The record cannot be written:** the command goes on without claiming the home, and warns.

`c2 home`, which shows the home's owner, and `c2 home release`, which removes the record, are [#403](https://github.com/jackgeek/climon2/issues/403).

## Considered options

- **Keep both hosts' sessions apart by recording each session's origin.** This was the first design of #133: discovery would judge only the sessions created on its own host, and keep the rest as "created elsewhere". Rejected. A host still could not attach to, kill or clean up the other's sessions, a transient Linux machine ID needed climon2 to keep an ID of its own in the shared home, keyed by host name, and a mistaken match could delete another host's live sessions. Working across machines belongs to `c2 relay`.
- **Leave shared homes unsupported, as before.** Rejected: nothing stopped a second host, which then deleted the first host's live sessions.

## Consequences

- Every record a reader sees was written by its own host, so stale cleanup judges each session's identity as before (ADR 0005), and discovery has no "created elsewhere" state. `daemon_origin` stays in the frozen metadata core, for diagnostics.
- Known limit: every Linux machine's initial PID namespace has the same inode, so two machines without a stable machine ID, or two clones with the same one, look like one host.
- A container's PID namespace ends with the container, and one started later has another, so a home a container owned refuses its successor until the record is released.
- The machine ID read from the system survives reboots on every platform, except a transient Linux one, which is never recorded.
