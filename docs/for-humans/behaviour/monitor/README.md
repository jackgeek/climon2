# Monitor

Agreed specifications of the monitor: what it shows, how it starts, stops and
fails, the resources it may use, and the evidence a release needs before it
ships.

**Audience:** Contributors.

## Shared terms

The monitor's main screen is the [selector](../../../../GLOSSARY.md#attachment-and-control):
a list of sessions the user picks from. Showing a session's terminal to the
user is [viewing](../../../../GLOSSARY.md#status-and-attention) it; typing into
it requires holding [control](../../../../GLOSSARY.md#attachment-and-control)
of that session.

## Contents

| Document | Purpose |
|---|---|
| [selector.md](selector.md) | The selector, its keys and help, and how attention is presented. |
| [lifecycle.md](lifecycle.md) | How the monitor starts, exits and handles failure. |
| [operational-limits.md](operational-limits.md) | The `monitor.limits` settings: memory, history, queues and admission. |
| [acceptance.md](acceptance.md) | The scenarios and evidence a release must pass before it is qualified. |
