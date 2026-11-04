# Client library embedding and lifetime

**Status:** agreed design, not implemented. The
[library decision](https://github.com/jackgeek/climon2/issues/65) owns these
requirements. Complete declarations, data layouts, result codes, native-path
encoding, and executable examples remain with
[interface schemas](https://github.com/jackgeek/climon2/issues/71).
Revision 1 is planned, not a published interface.

Use the [domain glossary](../../GLOSSARY.md), [technical terms](TERMS.md),
and [protocol overview](README.md) with this reference.

## Public interface

A [monitor](../../GLOSSARY.md#sessions) loads each
[session](../../GLOSSARY.md#sessions)'s exact-release
[client library](../../GLOSSARY.md#sessions) through a versioned
[C ABI](TERMS.md#c-abi). Instances use [opaque handles](TERMS.md#opaque-handle),
fixed-width values, explicit buffer lengths and ownership, and structured
errors. Rust data layouts and unwinding are never part of this interface.

One stable exported entry point, `climon2_client_get_interface`, takes a
requested interface revision. It returns that revision's
[function table](TERMS.md#function-table) and release identity, or an explicit
unsupported result. This describes the operation, not a complete C declaration.
Its exact arguments, ownership representation, and failure results must be
specified before implementation.

Freeze the bootstrap calling convention and each published revision's layout
and semantics. Incompatible changes require a new revision. A different release
may support the same revision; release identity alone does not establish
interface compatibility. The returned table and release metadata are immutable
and remain valid until process exit.

Validate both the requested interface and exact session-release identity before
instance creation or a [session daemon](../../GLOSSARY.md#sessions) connection.
Loading alone grants no
[control](../../GLOSSARY.md#attachment-and-control). Missing symbols, unsupported
interfaces, wrong releases, missing dependencies, and loader failures are
explicit availability errors, not evidence that a session ended.

## Artifact and dependency lookup

On macOS, load the absolute path
`<executable home>/<session version>/libclimon2_client.dylib` from the
[install layout](../../GLOSSARY.md#installation). Never substitute `current`,
another installed release, or a library found through a search path.
The [client executable](../../GLOSSARY.md#sessions) loads the same filename
beside itself and verifies its own exact release. Other platforms choose their
filenames when supported.

Load with eager symbol resolution and local symbol visibility. Bundled private
dependencies resolve relative to the library, not the current working directory
or ambient library search paths. Give them release-distinct loader identities
where needed to prevent cross-binding. System libraries remain system
dependencies. Simultaneous releases must not accidentally resolve one another's
private symbols or dependencies.

Installed libraries are trusted native code. Initializers execute before
identity validation; compatibility checks do not sandbox a malicious artifact.
A native crash during loading can terminate the whole monitor.

## Loading and instance lifetime

Perform loading and validation away from the rendering/input thread. Share one
load attempt per release and one loaded library among that release's instances.
A stalled load must not freeze global monitor actions or indefinitely prevent
other releases from starting.

Retain every successfully opened library handle until monitor process exit,
including handles rejected by later compatibility checks. Rejected libraries
create no instances. Do not unload during error recovery or run library
finalizers as a recovery mechanism. Loader failures still report errors.
This intentionally keeps opened releases in use until process exit, even after
their last instance ends. Releases remain additive and immutable; implementing
version cleanup is out of scope.

The monitor creates a dedicated thread for each live session's instance.
Create, run, and clean up that instance on its thread. The library owns its
connection, per-session state, and any internal workers. Only documented
control/communication operations may execute concurrently with that instance.
A separate thread-safe control handle lets the monitor request shutdown without
waiting for the running operation or thread to finish.

Complete instance and internal-worker cleanup before the client thread returns.
The monitor joins the thread before reporting completed shutdown. Release the
control handle only after its permitted concurrent uses end; exact handle
validity and release operations belong to the completed schemas. A final
[event](TERMS.md#event), connection closure, or daemon-accepted
[detach](../../GLOSSARY.md#attachment-and-control) is not proof of cleanup or
thread completion. Cleanup failure must be reported, not called success.

Never forcibly terminate a stalled thread or unload running code. The policy
when cleanup cannot finish belongs to the
[lifecycle decision](https://github.com/jackgeek/climon2/issues/61).
Load/start concurrency, progress deadlines, and admission/exhaustion policies
belong to [operational limits](https://github.com/jackgeek/climon2/issues/69).
This document assigns no numerical budgets.

## Per-instance inputs and memory

Supply explicit per-instance paths, validated settings snapshots, session
identity, and operation. Copy startup inputs before creation returns. Resolve
relative paths before creation and preserve
[native paths](TERMS.md#native-path) losslessly using length-delimited native
data, not [UTF-8](TERMS.md#unicode-and-utf-8)-only strings. Exact encoding
belongs to the completed schemas.

Do not treat process-global arguments, environment, settings, current directory,
signals, or [standard streams](TERMS.md#standard-streams) as private instance
inputs. Libraries must not mutate environment, current directory, signal
handlers, or standard streams. Diagnostics are instance-scoped;
the monitor owns its [host](../../GLOSSARY.md#sessions) terminal.

Each allocator frees its own allocations. Every later buffer must explicitly
use copying, call-scoped borrowing, or ownership transfer with a matching release
operation. Never retain a caller buffer by implication or free a library
allocation with another release's allocator. Any callback introduced by the
communication design must obey the same lifetime and no-unwinding rules.

Instance state is separate; function tables and release metadata are immutable.
Shared library resources must be thread-safe. No single-client process-global
lock or shared mutable per-session settings may serialize unrelated instances.
One stalled instance must not prevent other instances or releases from making
progress.

## Failures

Catch recoverable Rust panics inside every exported library entry and report an
explicit failure. For an instance failure, stop ordinary work on that instance;
permit only cleanup declared safe by its lifecycle. Other instances continue
where safe. No unwinding crosses the public interface in either direction.

This is not native-crash isolation. Abort, native faults, and panic-abort builds
can terminate the monitor and every hosted client. No new session-survival
guarantee exists: the daemon's existing
[leave policy](../../GLOSSARY.md#attachment-and-control) still applies.

## Required automated evidence

These are obligations for later implementation, not tests already run:

- Exercise packaged dynamic libraries through the public interface: multiple
  same-release instances and simultaneous different-release libraries.
- Verify exact artifact lookup, identity and revision validation before
  connection, and explicit missing/unsupported/mismatched artifact failures.
- Use different private dependency behavior in mixed-release fixtures to detect
  accidental symbol/dependency cross-binding.
- Cover native paths, independent paths/settings/diagnostics/resources, and
  continued progress when another instance or release stalls or fails.
- Check copying, borrowing and transferred-buffer lifetimes, allocator pairing,
  internal-worker cleanup, control-handle lifetime and actual thread joins,
  including partial startup and recoverable panics.
- Exercise native crash/abort in a disposable monitor process. Report
  whole-monitor failure exposure; do not assert process-isolated client
  survival or guaranteed session survival.

The [communication decision](https://github.com/jackgeek/climon2/issues/66)
still owns operation delivery and representation, ordering and correlation.
Lifecycle owns recovery and incomplete cleanup; operational limits own budgets;
interface schemas own exact declarations and valid call examples. These
decisions must preserve this embedding contract without inventing
[readiness](../../GLOSSARY.md#protocol-and-plugin-concepts) or successful cleanup
from library loading.
