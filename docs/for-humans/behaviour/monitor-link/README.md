# Monitor links

Agreed specifications of monitor links: how a monitor on one machine sees and
drives the sessions on another, and the evidence a release needs before it
ships them.

**Builds on:** [monitor](../monitor/README.md)

**Audience:** Contributors.

## Shared terms

A [monitor link](../../../../GLOSSARY.md#monitor-links) is a connection,
authenticated and encrypted by climon2 itself, between two programs on
different machines. The [relay](../../../../GLOSSARY.md#monitor-links) serves
its machine's sessions. The
[hub monitor](../../../../GLOSSARY.md#monitor-links) is a monitor that
presents and drives them. A session the hub monitor reaches through a monitor
link is a [remote session](../../../../GLOSSARY.md#monitor-links); one on its
own machine is a [local session](../../../../GLOSSARY.md#monitor-links).

The two ends first trust each other through a
[link invitation](../../../../GLOSSARY.md#monitor-links): the relay shows a
short one-time number, the **PIN**, and the hub monitor's user types it in.
That exchange is **pairing**. Each end then remembers the other's public key,
so later links need no PIN, until one end **forgets** the other.

## Contents

| Document | Purpose |
|---|---|
| [acceptance.md](acceptance.md) | The scenarios, adversarial evidence and recorded fixtures a release must pass before monitor links are qualified. |
