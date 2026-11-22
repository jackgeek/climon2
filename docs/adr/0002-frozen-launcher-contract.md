# Frozen, dependency-free launcher contract

The launcher is the one file shared by every installed version. Because sessions must survive upgrades, its contract is frozen forever: read `<climon2 executable home>/current` (one line, naming a version folder), then run `<climon2 executable home>/<version>/climon2-cli` with the arguments unchanged. Any launcher can therefore drive any version, older or newer. The launcher has no dependencies at all, not even workspace crates (enforced by a test; see ADR 0004), which keeps its attack surface small and reduces the need to replace it.

On Windows the contract has two additions, fixed before the first Windows release because they can never change afterwards:

- The launcher runs `<version>\climon2-cli.exe`.
- When `current` is missing or access to it is denied, the launcher retries for up to one second before failing. An installer replaces `current` by renaming a new file over it, and measurement showed that a Windows reader can find the file missing for about a tenth of a second during that rename ([Windows install probes](https://github.com/jackgeek/climon2/blob/a31b371d34db3036224caa5b2c2c7ac470b4836d/docs/research/windows-install-probes.md)).

## Considered options

- **Version-specific launcher, replaced on every install:** rejected. On Windows a running launcher is locked, so replacing it would mean stopping live sessions on every upgrade.
- **Launcher with richer logic (version selection, config):** rejected. Every feature added to the launcher becomes part of a contract that can never change.
- **Windows: ask the user to stop the sessions holding the launcher:** rejected. This was the original plan, deferred to Windows support. It interrupts work to deliver a file whose behaviour is the same in every version.
- **Windows: rename the running launcher aside and clean up later:** rejected. It works (a running program can be renamed, though not deleted until it exits), but it leaves files behind that every later install must sweep, again to deliver a file that rarely changes.

## Consequences

- On macOS and Linux the launcher `exec`s into the CLI, so no session holds it open, and the installer always replaces it atomically (temp then rename).
- On Windows the launcher stays running and locks its file, so replacement is best effort. The installer skips an identical launcher, replaces a different one when no launcher is running, and otherwise keeps the old one and still succeeds. A changed launcher arrives with the first install made while none is running. [Windows packaging and upgrades](../for-humans/windows/distribution.md) has the detail.
- New launcher behaviour belongs in the CLI, never in the launcher.
