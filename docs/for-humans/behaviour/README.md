# Behaviour

Agreed specifications of what climon2 does: how a session reports its status,
and how the monitor presents, manages and limits sessions.

**Audience:** Contributors.

## Shared terms

A session has a [session status](../../../GLOSSARY.md#status-and-attention):
whether its command is active or idle, and whether it needs the user's
[attention](../../../GLOSSARY.md#status-and-attention). The session daemon
works the status out from the command's output; clients and the monitor
present it. Each specification here opens with how much of it is implemented.

## Contents

| Document or folder | Purpose |
|---|---|
| [session-status.md](session-status.md) | How a session daemon detects activity and attention, and how clients learn of changes. |
| [monitor/](monitor/README.md) | The monitor's selector, lifecycle, operational limits and acceptance criteria. |
