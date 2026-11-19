# climon2 documentation

Everything about climon2 written for people to read: how it is released, how
it behaves, the protocols between its programs, and how its code is arranged.
Architecture decisions live in [`docs/adr/`](../adr/) and agent instructions in
[`docs/agents/`](../agents/), outside this tree.

**Audience:** Users, Integrators and Contributors.

- **Users** run climon2.
- **Integrators** write clients or plugins against its protocols.
- **Contributors** change climon2's code.

## Shared terms

climon2 runs each command in a [session](../../GLOSSARY.md#sessions): one run
of a command inside a terminal that climon2 owns. A detached background
process, the [session daemon](../../GLOSSARY.md#sessions), owns that terminal.
A [client](../../GLOSSARY.md#sessions) connects the user's own terminal, the
[host](../../GLOSSARY.md#sessions), to a session daemon. The
[monitor](../../GLOSSARY.md#sessions) is a program that shows one session at a
time and switches between sessions through clients it owns. The
[domain glossary](../../GLOSSARY.md) defines every other climon2 concept.

Many documents here are agreed **specifications**: each opens with a status
line saying how much of it is implemented, and links the issue that settled it.

## Contents

| Document or folder | Purpose |
|---|---|
| [distribution.md](distribution.md) | Supported platforms, release archives and how a release is built. |
| [behaviour/](behaviour/README.md) | Specifications of what sessions and the monitor do. |
| [protocol/](protocol/README.md) | The interfaces between climon2's programs, for plugin and client authors. |
| [architecture/](architecture/README.md) | Diagrams of climon2's executables, libraries and crates. |
| [check.py](check.py) | CI check that this tree follows its own rules: no orphans, complete tables, allowed links. |
