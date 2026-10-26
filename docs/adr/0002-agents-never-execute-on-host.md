# Agents never execute code on the host

Agents get no shell, network, credentials or built-in Copilot tools, only path-scoped file tools that the factory enforces. Agent-written code runs only in three places:
- fresh, offline local Rancher Desktop containers;
- GitHub Actions, for declared candidates and releases;
- natively on the Mac, after the user approves that specific commit.

We chose local containers over GitHub for the inner loop because GitHub builds are expensive. A full sandbox can't validate macOS PTY behaviour, and no macOS VM option runs on this Intel Mac with the current macOS. See `docs/research/macos-execution-isolation.md`.

## Consequences

- macOS validation is gated on approval and cannot be unattended until a macOS VM is evaluated.
- The Windows inner loop only cross-compiles (GNU). Real Windows behaviour is checked on GitHub for declared candidates.
