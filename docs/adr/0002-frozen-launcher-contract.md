# Frozen, dependency-free launcher contract

The launcher is the one file shared by every installed version. Because sessions must survive upgrades, its contract is frozen forever: read `<climon2 executable home>/current` (one line, naming a version folder), then run `<climon2 executable home>/<version>/climon2` with the arguments unchanged. Any launcher can therefore drive any version, older or newer. The launcher uses only `std` (no crates.io dependencies, enforced by a test), which keeps its attack surface small and reduces the need to replace it.

## Considered options

- **Version-specific launcher, replaced on every install:** rejected. On Windows a running launcher is locked, so replacing it would mean stopping live sessions on every upgrade.
- **Launcher with richer logic (version selection, config):** rejected. Every feature added to the launcher becomes part of a contract that can never change.

## Consequences

- On macOS the launcher `exec`s into the CLI, so no session holds it open, and the installer always replaces it atomically (temp then rename).
- On Windows, where the launcher stays running and locks its file, the installer has to list the sessions holding it and ask the user to stop them before replacing it. That is deferred to Windows support.
- New launcher behaviour belongs in the CLI, never in the launcher.
