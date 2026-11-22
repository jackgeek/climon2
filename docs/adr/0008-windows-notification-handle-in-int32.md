# Windows notification handle in the frozen int32_t output

Native revision 1's `notification(communication, int32_t *out)` returns a pollable descriptor: an `eventfd` on Linux. On Windows the library's notification is an unnamed manual-reset event, so the output must carry a `HANDLE`, which is pointer-sized. We carry it in the existing `int32_t` output, and the caller sign-extends it back to a `HANDLE`. Microsoft documents that 64-bit Windows kernel handles have only 32 significant bits and can be truncated and sign-extended safely, so one header and one function-table layout serve every platform. This is decided in [#216](https://github.com/jackgeek/climon2/issues/216) and specified in [native revision 1](../for-humans/protocol/monitor-client/README.md#startup-limits-and-resources). It is implemented in [#318](https://github.com/jackgeek/climon2/issues/318).

## Considered Options

- **A Windows-only `#ifdef` that makes the output a `HANDLE *` (or `uintptr_t *`).** Rejected. It splits the frozen header by platform, so every C consumer, the layout check and the Rust declarations would carry two signatures for one call, to make room for bits Windows never uses.
- **A socket the monitor could poll.** Rejected. Windows AF_UNIX has no `socketpair`, so the library would need a loopback connection per instance just to signal readiness.
- **An I/O completion port.** Rejected. A completion port cannot be shared between the library and the monitor's own wait.

## Consequences

- The value is a handle, not a descriptor: the caller waits on it and never sets, resets or closes it. Reading it as a C runtime file descriptor is a bug.
- "Nonnegative" in the call's contract still holds, because handle values are small multiples of four. Failure still writes -1.
- The handle is frozen with revision 1. Changing the output type later needs a new revision.
