# Architecture

Diagrams of what climon2 is built from: the executables and dynamic libraries
that run, and the workspace crates that build them.

**Audience:** Contributors.

## Shared terms

Every diagram in both documents draws **consumption**: an arrow runs from the
thing that uses another to the thing it uses, never from an installer to what
it installs.
Each diagram states its own scope above it and describes its nodes in a table
below it. When a change adds, removes or rewires an artifact or a crate, the
same change updates the matching diagram.

## Contents

| Document | Purpose |
|---|---|
| [runtime-artifacts.md](runtime-artifacts.md) | The executables and dynamic libraries a `c2` command runs, and what each loads or starts. |
| [crate-relationships.md](crate-relationships.md) | Every workspace crate and its dependencies, split into shipped and development views. |
