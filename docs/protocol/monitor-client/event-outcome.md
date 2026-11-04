# Client outcome and failure

**Status:** hosting failure boundary agreed; lifecycle/interface details unresolved.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

When communication remains possible, the [client](../../../GLOSSARY.md) reports
an explicit completion or failure to the [monitor](../../../GLOSSARY.md).
A final notification alone proves neither daemon-accepted
[detach](../../../GLOSSARY.md) nor completed worker/resource cleanup.
Observe client thread completion and resource lifetime separately.

Native crashes or aborts can terminate the monitor and every hosted client.
Ordinary errors and recoverable panics should remain local where possible,
without claiming successful cleanup or detach after failure. Session survival
is not guaranteed by this hosting model.

[Library lifetime](https://github.com/jackgeek/climon2/issues/65) must specify
panic containment, instance/thread cleanup, and loaded-library safety.
[Lifecycle](https://github.com/jackgeek/climon2/issues/61) owns recovery, terminal
restoration, and visible failures.
[Communication](https://github.com/jackgeek/climon2/issues/66) and
[interface schemas](https://github.com/jackgeek/climon2/issues/71) must assign
outcome types, reporting paths, and examples.
[Operational limits](https://github.com/jackgeek/climon2/issues/69) owns
startup/stall/shutdown deadlines, including clients that cannot join cleanly.
