# One home, one host

A climon2 home belongs to one host: one machine, or on Linux without a stable machine ID, one PID namespace. A session's PID and process identity mean something only there, so a home shared with another machine (a network home directory) or another container would let one host take the other's live sessions for ended ones, or never clean up after them. We decided that climon2 refuses such a home instead of trying to keep both hosts' sessions apart. Working across machines is planned as `c2 relay`, which does not exist yet. It is decided in [#133](https://github.com/jackgeek/climon2/issues/133) (owner, 2026-11-26), and supersedes the cross-machine parts of [ADR 0003](0003-platform-and-shell-seams.md) and [ADR 0005](0005-frozen-session-metadata-core.md).

## The home owner

The home records its owner in `<climon2 home>/owner`, a JSON object holding the owner's origin, as a session's `daemon_origin` does (ADR 0005):

```json
{"machine_id":"8f2c4d1e9a7b4c3d8e6f5a4b3c2d1e0f","pid_namespace":4026531836}
```

- `machine_id` is the machine's stable identity: on Linux systemd's `/etc/machine-id`, on macOS the hardware UUID from `gethostuuid`, on Windows `MachineGuid`. On Linux it is absent when `/etc/machine-id` is missing, and when it does not survive a reboot: when the last mount in `/proc/self/mountinfo` that holds it (at `/etc/machine-id`, `/etc` or `/`) is a `tmpfs` or an `overlay`. That covers the transient ID systemd mounts from `/run/machine-id`, and the tmpfs or overlay root of a live system, which writes a fresh ID at every boot (decided 2026-11-26). Only 32 hex digits are a machine ID: anything else, such as the `uninitialized` a first boot writes, is none. A container's bind mount of its host's real file is a mount of the host's file system, so it stays stable.
- `pid_namespace` is Linux only: the inode of `/proc/self/ns/pid`.
- At least one member is present. Unknown members are ignored, so later releases can add to the record. A record that is not such an object, or is not a plain file, is refused and never replaced.

Two origins are the **same host** when both have a stable machine ID and the IDs are equal, whatever their PID namespaces: WSL2, LXC/LXD, systemd-nspawn and Flatpak get a new PID namespace inode at every start, so comparing namespaces too refused the same machine after a restart (decided 2026-11-26). Only when either side has no stable machine ID do the PID namespaces decide. On macOS and Windows, which always have one, the machine ID alone decides.

## The check

`exec`, `attach`, `list`, `kill`, the monitor (through `c2 monitor`), and the session daemon when it starts, each check the home before they do anything else with it. `--help` and `--version` do not. The check lives in one place, `climon2_platform::owner::check`. `c2 home` and `c2 home release` skip it (below).

- **No owner record:** the check claims the home for this host. It writes the record to an owner-only file of its own, named by the process and a random number so nothing a crash left behind is in the way, and hard-links it into place, so the record appears whole and once. Where the file system cannot link, it creates the record in place exclusively instead, so racing claims still agree, though one may briefly meet a half-written record and refuse. When several first commands race, the one whose record is in place owns the home, and the others are judged against it. A home with no record after an upgrade is claimed the same way, without a prompt.
- **This host is the owner:** the command goes on.
- **Another host is the owner:** the command exits 125 before any session is started, attached, listed or killed, with: "this climon2 home is in use by another machine or container (<owner origin>). climon2 does not work across machines yet (`c2 relay` is planned). To move this home here, run `c2 home release`, or `c2 home release --force` if the host that owns it is gone. To use it from here anyway, unclaimed, give the command `--force`." A daemon reports a short static form of it to the CLI that started it and logs the full refusal.
- **This host cannot read its own origin:** with no owner record, the command goes on without claiming the home, and warns. With one, it refuses, saying the origin could not be checked.
- **The record cannot be read, or is not valid:** the command refuses with 125, naming the record and saying that `c2 home release` (with `--force` if the home still holds sessions), or deleting `<home>/owner`, lets the next command claim the home afresh, and that the command's `--force` uses it anyway. The record is left as it is.
- **`--force`** (decided 2026-11-26): `exec`, `attach`, `kill`, `list` and `monitor` take `--force`, once, with no short form: among `exec`'s leading options, in any order with `--headless` and `--nested` (a repeated one is the command, as for every `exec` option), and as the first argument of the others. The owner check, forced or not, runs before `exec`'s nested-session prompt. It skips a foreign or unreadable record: the command uses the home without claiming it and prints a warning naming the owner, `climon2: warning: --force uses this climon2 home without claiming it: <refusal>`. `exec --force` passes it on to the daemon as `--home-owner forced`, so the daemon skips the same refusals and logs them. A host that cannot read its own origin is still refused.
- **The record cannot be written:** the command goes on without claiming the home, and warns (decided 2026-11-26).

## Showing and releasing the owner

Decided with [#403](https://github.com/jackgeek/climon2/issues/403), which this decision folds in. Neither command checks the owner first: they are how a refusal is diagnosed, and how a home is moved.

- `c2 home` (`h`) prints the home's path, its owner (or that it has none, or why its record cannot be read), and this host's origin with whether it is the owner. It claims nothing and exits 0.
- `c2 home release` removes `<home>/owner`, whoever it names, so the next command claims the home afresh, and prints whose record it removed. A record that cannot be read, or a link at its name, is removed itself; a link is never followed. An empty folder at the name is removed too; any other folder is left, and the message says to delete it by hand. Before it removes anything it makes a discovery pass that removes no session metadata (decided 2026-11-26):
  - **On the owner,** it refuses with 125 while any of the owner's sessions is live or not proven ended (its daemon still runs here, or its identity cannot be checked). Sessions another host created do not count. `--force` releases it anyway, with a warning naming how many may run (decided 2026-11-26).
  - **On any other host,** or one that cannot tell whose the home is (its own origin or the record cannot be read), it refuses with 125 while the home holds any session record at all, since the owning host may still be running them. `c2 home release --force` releases it anyway, for when the owning host is gone. The refusals of a foreign or unreadable record say so.
- **After a forced release** the new owner claims a home that may hold the earlier owner's sessions. Discovery never proves a session gone whose `daemon_origin` is another host's: it is uncertain, created elsewhere, and kept, never deleted or signalled. A record without an origin is judged as before. So a host that lost its home can release it with `--force`, claim it again and manage its sessions.

## Considered options

- **Keep both hosts' sessions apart by recording each session's origin.** This was the first design of #133: discovery would judge only the sessions created on its own host, and keep the rest as "created elsewhere". Rejected. A host still could not attach to, kill or clean up the other's sessions, a transient Linux machine ID needed climon2 to keep an ID of its own in the shared home, keyed by host name, and a mistaken match could delete another host's live sessions. Working across machines belongs to the planned `c2 relay`.
- **Compare PID namespaces as well as machine IDs.** Rejected (2026-11-26): the same machine gets a new namespace on every start of WSL2, LXC/LXD, systemd-nspawn or Flatpak, and was refused by its own home.
- **Leave shared homes unsupported, as before.** Rejected: nothing stopped a second host, which then deleted the first host's live sessions.

## Consequences

- Every record a reader sees was written by its own host, or by the host it took the home from with `--force`, so stale cleanup judges each session's identity as before (ADR 0005), except that a record whose `daemon_origin` is another host's is kept as created elsewhere.
- Known limits, accepted (2026-11-26):
  - A Flatpak app, or a container that bind-mounts the host's `/etc/machine-id`, has the host's machine ID, so it passes the owner check, but it cannot see the host's processes, so it may judge the host's live sessions as ended and clean them up.
  - A host without a stable machine ID matches an owner by PID namespace alone, so it matches an owner in the initial PID namespace even when that owner has a machine ID.
- Known limit: every Linux machine's initial PID namespace has the same inode, so two machines without a stable machine ID, or two clones with the same machine ID, look like one host. So can two containers without one on different machines: each kernel numbers PID namespaces from the same range of inodes. Containers that share their host's machine ID (a bind mount of `/etc/machine-id`) are their host even in another PID namespace, where they cannot see the host's processes, so they can take the host's live sessions for ended ones, as before #133.
- A container without a stable machine ID gets a new PID namespace when it starts again, so a home it owned refuses its successor until the record is released.
- The machine ID read from the system survives reboots on every platform, except a transient Linux one, which is never recorded.
