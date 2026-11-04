# Helper outcome and failure notification

**Status:** agreed observations; wire names and [payload](../TERMS.md#payload) [schemas](../TERMS.md#schema) pending;
not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

When orderly communication remains possible, the [helper](../../../GLOSSARY.md) reports an explicit
terminal outcome through structured messages. This page covers the agreed
outcome/failure obligation; it does not assign a single [event](../TERMS.md#event) name or exclude
separate message types in the completed schema.

The [monitor](../../../GLOSSARY.md) independently observes protocol state, [EOF](../TERMS.md#eof), and process exit.
A final message alone does not prove normal cleanup or clean exit.
Normal library return requires workers and resources to finish cleanup;
incomplete cleanup is a helper failure, not successful [detach](../../../GLOSSARY.md).

Loader failures before communication use [stderr](../TERMS.md#standard-streams) and the [climon2 error code](../../../GLOSSARY.md).
Native crashes may make a final message impossible. The monitor and other
helpers must remain operational without assuming the affected [session](../../../GLOSSARY.md) survived.

## Schema pending

Helper-specific fields are tracked by
[schema completion](https://github.com/jackgeek/climon2/issues/71).

Exact event names, outcome/error fields, exit-status representation, and valid
wire examples are not yet assigned. Apply the
[event envelope](event.md) wherever an event is chosen.
[Lifecycle](https://github.com/jackgeek/climon2/issues/61) owns recovery and visible
behavior; [operational limits](https://github.com/jackgeek/climon2/issues/69) owns
shutdown/stall deadlines.

Use [common failure rules](../README.md#compatibility-errors-and-limits).
Related: [response](response.md), [detach](request-detach.md), and
[session end](event-session-ended.md).
**Source:** [helper resolution](https://github.com/jackgeek/climon2/issues/66#issuecomment-6616062936).
