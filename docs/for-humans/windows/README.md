# Windows

Agreed specifications of the Windows port: how climon2's processes start, run
and end on Windows, what a Windows release contains, and what must be tested
before one is published. Each opens with how much of it is built.

**Audience:** Contributors.

## Shared terms

On Windows a [session daemon](../../../GLOSSARY.md#sessions) runs the
session's command in a pseudoconsole, the Windows counterpart of a Unix
pseudo-terminal. climon2 ships its own copy of ConPTY, the Windows component
that provides it. Each document below introduces the rest of its own terms.

## Contents

| Document | Purpose |
|---|---|
| [lifecycle.md](lifecycle.md) | How the daemon, the command, the hand-offs and the client's console start, run and end on Windows. |
| [distribution.md](distribution.md) | What a Windows release contains, how it is installed and what an upgrade may touch. |
| [qualification.md](qualification.md) | The terminals, shells, scenarios and manual checks a Windows release must pass. |
| [quality.md](quality.md) | The CI jobs that check Windows code and the quality thresholds its source meets. |
