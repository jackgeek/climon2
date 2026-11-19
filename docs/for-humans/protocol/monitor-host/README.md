# Monitor to host

The bytes the monitor writes to its host terminal to advertise whether a
session needs [attention](../../../../GLOSSARY.md#status-and-attention), and to
ring the bell. The shared rules are in the overview's
[monitor-to-host section](../README.md#monitor-to-host-status-and-bell).

**Audience:** Integrators and Contributors.

## Shared terms

A session needs attention when its command is waiting for the user. The
status reports are
[OSC](../TERMS.md#osc) 7501 escape sequences: the host reads them to show
whether any session the monitor watches needs attention. A
[toast](../TERMS.md#toast) is a short message the monitor draws over its
screen; the bell is a single [BEL](../TERMS.md#bel) byte it writes as one
appears.

## Contents

| Document | Purpose |
|---|---|
| [report-attention.md](report-attention.md) | Report that a connected session needs attention. |
| [report-monitoring.md](report-monitoring.md) | Report continued monitoring with no session needing attention. |
| [report-clear.md](report-clear.md) | Clear the monitor's status advertising when it exits. |
| [bell.md](bell.md) | Ring one host bell per toast the monitor presents. |
