# Stable launcher with self-contained versioned runtimes

Climon2 ships four executables in this install layout:

```
<install>/climon2              launcher (stable)
<install>/current              one line: the current runtime version
<install>/versions/<v>/climon2-cli
<install>/versions/<v>/climon2-daemon
<install>/versions/<v>/climon2-client
```

The `climon2` launcher reads `current` and runs that version's CLI. On Unix it calls `exec` to replace itself. On Windows it spawns the CLI in a kill-on-close job object, ignores Ctrl+C itself, and propagates the CLI's exit code.

New sessions start with the CLI's own runtime version. Attaching uses the client from the same version as the session's daemon, so a client and its daemon are always the same version. Upgrading means adding a version folder and atomically replacing `current`. Running sessions keep their old version until they end.

The same launcher-based layout is used on every platform, instead of a symlink on Unix. This keeps upgrades in place on Windows, where running executables can't be overwritten and creating symlinks requires privileges.

## Consequences

Four surfaces must stay backward compatible, and each is versioned:
- the session metadata format (`format_version`);
- the hello handshake;
- the client launch interface, `climon2-client --protocol <n> attach --metadata <path>`;
- the launcher contract: how it reads `current` and forwards arguments, environment, stdio and exit codes.

Release gates include cross-version tests from 0.2.0 onwards. The launcher changes only through an explicit installer step.
