# One home, one host

A climon2 home belongs to one host: one machine, or on Linux without a stable machine ID, one PID namespace. A session's PID and process identity mean something only there, so a home shared with another machine (a network home directory) or another container would let one host take the other's live sessions for ended ones, or never clean up after them. We decided that climon2 refuses such a home instead of trying to keep both hosts' sessions apart. Working across machines is planned as `c2 relay`, which does not exist yet. It is decided in [#133](https://github.com/jackgeek/climon2/issues/133) (owner, 2026-11-26), and supersedes the cross-machine parts of [ADR 0003](0003-platform-and-shell-seams.md) and [ADR 0005](0005-frozen-session-metadata-core.md).

## The home owner

The home records its owner in `<climon2 home>/owner`, a JSON object holding the owner's origin, as a session's `daemon_origin` does (ADR 0005):

```json
{"machine_id":"8f2c4d1e9a7b4c3d8e6f5a4b3c2d1e0f","pid_namespace":4026531836}
```

- `machine_id` is the machine's stable identity: on Linux systemd's `/etc/machine-id`, on macOS the hardware UUID from `gethostuuid`, on Windows `MachineGuid`. On Linux it is absent when `/etc/machine-id` is missing, and when it does not survive a reboot: when the last mount in `/proc/self/mountinfo` that holds it (at `/etc/machine-id`, `/etc` or `/`) is a `tmpfs` or an `overlay`. That covers the transient ID systemd mounts from `/run/machine-id`, and the tmpfs or overlay root of a live system, which writes a fresh ID at every boot (decided 2026-11-26). A container's bind mount of its host's real file is a mount of the host's file system, so it stays stable.
- `pid_namespace` is Linux only: the inode of `/proc/self/ns/pid`.
- At least one member is present. Unknown members are ignored, so later releases can add to the record. A record that is not such an object, or is not a plain file, is refused and never replaced.

Two origins are the **same host** when both have a stable machine ID and the IDs are equal, whatever their PID namespaces: WSL2, LXC/LXD, systemd-nspawn and Flatpak get a new PID namespace inode at every start, so comparing namespaces too refused the same machine after a restart (decided 2026-11-26). Only when either side has no stable machine ID do the PID namespaces decide. On macOS and Windows, which always have one, the machine ID alone decides.

## The check

`exec`, `attach`, `list`, `kill`, the monitor (through `c2 monitor`), and the session daemon when it starts, each check the home before they do anything else with it. `--help` and `--version` do not. The check lives in one place, `climon2_platform::owner::check`. `c2 home` and `c2 home release` skip it (below).

- **No owner record:** the check claims the home for this host. It writes the record to an owner-only file of its own and hard-links it into place, so the record appears whole and once. When several first commands race, the one whose record is in place owns the home, and the others are judged against it. A home with no record after an upgrade is claimed the same way, without a prompt.
- **This host is the owner:** the command goes on.
- **Another host is the owner:** the command exits 125 before any session is started, attached, listed or killed, with: "this climon2 home is in use by another machine or container (<owner origin>). climon2 does not work across machines yet (`c2 relay` is planned). To move this home here, run `c2 home release`." A daemon reports a short static form of it to the CLI that started it and logs the full refusal.
- **This host cannot read its own origin:** with no owner record, the command goes on without claiming the home, and warns. With one, it refuses, saying the origin could not be checked.
- **The record cannot be read, or is not valid:** the command refuses with 125, naming the record and saying that `c2 home release`, or deleting `<home>/owner`, lets the next command claim the home afresh. The record is left as it is.
- **The record cannot be written:** the command goes on without claiming the home, and warns (decided 2026-11-26).

## Showing and releasing the owner

Decided with [#403](https://github.com/jackgeek/climon2/issues/403), which this decision folds in. Neither command checks the owner first: they are how a refusal is diagnosed, and how a home is moved.

- `c2 home` (`h`) prints the home's path, its owner (or that it has none, or why its record cannot be read), and this host's origin with whether it is the owner. It claims nothing and exits 0.
- `c2 home release` removes `<home>/owner`, whoever it names, so the next command claims the home afresh. It runs on any host, which is how a home is recovered when its owner is gone, and prints whose record it removed. A record that cannot be read, or a link at its name, is removed itself; a link is never followed. It refuses with 125 while this host has a live session in the home, found by a discovery pass that removes no stale metadata.

## Considered options

- **Keep both hosts' sessions apart by recording each session's origin.** This was the first design of #133: discovery would judge only the sessions created on its own host, and keep the rest as "created elsewhere". Rejected. A host still could not attach to, kill or clean up the other's sessions, a transient Linux machine ID needed climon2 to keep an ID of its own in the shared home, keyed by host name, and a mistaken match could delete another host's live sessions. Working across machines belongs to the planned `c2 relay`.
- **Compare PID namespaces as well as machine IDs.** Rejected (2026-11-26): the same machine gets a new namespace on every start of WSL2, LXC/LXD, systemd-nspawn or Flatpak, and was refused by its own home.
- **Leave shared homes unsupported, as before.** Rejected: nothing stopped a second host, which then deleted the first host's live sessions.

## Consequences

- Every record a reader sees was written by its own host, so stale cleanup judges each session's identity as before (ADR 0005), and discovery has no "created elsewhere" state. `daemon_origin` stays in the frozen metadata core, for diagnostics.
- Known limit: every Linux machine's initial PID namespace has the same inode, so two machines without a stable machine ID, or two clones with the same machine ID, look like one host. Containers that share their host's machine ID (a bind mount of `/etc/machine-id`) are their host even in another PID namespace, where they cannot see the host's processes, so they can take the host's live sessions for ended ones, as before #133.
- A container without a stable machine ID gets a new PID namespace when it starts again, so a home it owned refuses its successor until the record is released.
- The machine ID read from the system survives reboots on every platform, except a transient Linux one, which is never recorded.
