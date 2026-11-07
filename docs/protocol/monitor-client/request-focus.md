# Application focus

**Status: agreed, unimplemented #63 core operation; native declarations pending #71.**
The [monitor](../../../GLOSSARY.md#sessions) sends desired application focus
through its [client library](../../../GLOSSARY.md#sessions) to the
[session daemon](../../../GLOSSARY.md#sessions).
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
[technical terms](../TERMS.md), and inherited
[communication contract](../README.md#shared-in-process-communication-contract).

This uses a dedicated tagged [C-compatible](../TERMS.md#c-abi) core record, not
an [extension](../../../GLOSSARY.md#protocol-and-plugin-concepts), generic
injected Input, terminal reply, or serialized [envelope](../TERMS.md#envelope).
Semantic arguments identify the current
[connection generation](../TERMS.md#connection-generation),
[request ID](../TERMS.md#request-id),
[control period](../../../GLOSSARY.md#attachment-and-control),
[restoration ID](../../../GLOSSARY.md#protocol-and-plugin-concepts), and boolean
`focused`. Exact native tags, widths, declarations, errors, and executable C
examples remain [#71](https://github.com/jackgeek/climon2/issues/71).
The [wire Focus page](../daemon-client/request-focus.md) owns the portable
`0x39` [schema](../TERMS.md#schema), `focus-v1`
[capability](../../../GLOSSARY.md#sessions), results, errors and valid wire examples;
these are not native ABI declarations.

Require explicit optional core focus support through both library and daemon.
Unsupported focus leaves sessions usable with a visible limitation, never a
silent claim of support or a release-number inference.
[Plugin](../../../GLOSSARY.md#protocol-and-plugin-concepts) schemas and
negotiation remain deferred.

Submit only under current [controller](../../../GLOSSARY.md#attachment-and-control)/
[restoration](../../../GLOSSARY.md#restoration-and-presentation)/
[readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts) authority.
Selected ready [host](../../../GLOSSARY.md#sessions)-focused sessions without
modal [UI](../TERMS.md#ui) are keyboard-focused;
normal scrollback and nonmodal [toasts](../TERMS.md#toast) do not change that state.
Switch/menu opening sends focus-out when valid. A retained background controller
may send this non-user focus-out, not application input.
During restoration retain latest local desired focus and synchronize it when
newly ready; never queue old-state requests for future authority.
Host focus-in alone does not acknowledge
[attention](../../../GLOSSARY.md#status-and-attention).

Local admission proves only copied queuing, not daemon state/forwarding
acceptance. Recheck generation and authority at execution; explicit rejection
for known unexecuted stale work, truthful failure/known effects, and
[indeterminate](../TERMS.md#indeterminate-outcome) unknown execution follow shared
correlation rules, without replay. Eventual success distinguishes accepted focus
state from zero or three non-user [bytes](../TERMS.md#byte) accepted for forwarding;
it proves no command consumption or [PTY](../TERMS.md#pseudo-terminal-pty) read
and never counts as real user input.

Semantic example: ready selected session becomes hidden behind a modal menu.
Report [viewing](../../../GLOSSARY.md#status-and-attention) false and focus false
without releasing control. If application
focus reporting is off, daemon accepts false with zero forwarded bytes.
When the application enables reporting it emits current known focus-out once.
Closing the menu after readiness synchronizes current local focus; any separate
explicit acknowledgement still requires unobstructed viewing.
See the [canonical focus contract](../terminal-integration.md#host-focus-and-application-focus),
[Ready](request-ready.md), [Viewing](request-viewing.md), and
[Acknowledge](request-acknowledge.md).
