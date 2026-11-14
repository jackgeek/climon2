# Client library embedding and lifetime

**Status:** agreed design. Only the library side of the bootstrap is
implemented: every installed version ships its
[client library](../../GLOSSARY.md#sessions) file, exporting
only `climon2_client_get_interface`
([#75](https://github.com/jackgeek/climon2/issues/75)). The one shared loader
that the [monitor](../../GLOSSARY.md#sessions) and the
[client executable](../../GLOSSARY.md#sessions) will both use is implemented
([#84](https://github.com/jackgeek/climon2/issues/84)): it performs the
[lookup](#artifact-and-dependency-lookup), the load-failure classification and
the description validation below, and keeps every handle it opens. Only test
tooling uses it so far; loading by the monitor and the client executable is
not implemented. The
[library decision](https://github.com/jackgeek/climon2/issues/65) owns these
requirements; the [Linux decision](https://github.com/jackgeek/climon2/issues/125)
adds x86-64 GNU/Linux. [Native revision 1](monitor-client/core-v1.md), confirmed in
[#71](https://github.com/jackgeek/climon2/issues/71), publishes complete declarations,
[native-path](TERMS.md#native-path) encoding, result codes and compilable examples
as a specification, not a shipped library.

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
unsupported result. The [header](monitor-client/client-v1.h) fixes its signature;
[bootstrap rules](monitor-client/core-v1.md#representation-and-compatibility)
fix output ownership, failure results and resource qualification.

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

Load the absolute path `<executable home>/<session version>/<library file>`
from the [install layout](../../GLOSSARY.md#installation). The library file is
`libclimon2_client.dylib` on macOS and `libclimon2_client.so` on x86-64
GNU/Linux; the version folder, not the file name, carries the release. Never
substitute `current`, another installed release, or a library found through a
search path. The [client executable](../../GLOSSARY.md#sessions) loads the same
filename beside itself and verifies its own exact release.

On macOS, [dyld](TERMS.md#dynamic-loader) searches each `DYLD_LIBRARY_PATH`
directory for the library file's name before it opens an absolute path, so
that variable can substitute another file, such as another release's. After
loading, and before anything else, the loader asks `dladdr` which file holds
the loaded image's `climon2_client_get_interface`; an image without that
symbol is the requested file only if dyld lists that path among its images.
When the loaded image is not the requested path, the loader refuses the
substitute with an explicit error, rather than blaming the requested file for
a missing symbol, and keeps its handle loaded, like every opened handle (owner
decisions on [#84](https://github.com/jackgeek/climon2/issues/84)). The working
directory, `current` and `DYLD_FALLBACK_LIBRARY_PATH` never change which file
an absolute path loads.

Load with eager symbol resolution and local symbol visibility. Bundled private
dependencies resolve relative to the library, not the current working directory
or ambient library search paths. Give them release-distinct loader identities
where needed to prevent cross-binding. System libraries remain system
dependencies. Simultaneous releases must not accidentally resolve one another's
private symbols or dependencies.

On Linux, open the library with `dlopen(path, RTLD_NOW | RTLD_LOCAL)`. The
[dynamic loader](TERMS.md#dynamic-loader) can reuse an already loaded library
by name, [soname](TERMS.md#soname) or file, and same-named dependencies can
bind across releases even with local visibility. Each release's library is
therefore one self-contained file: it exports only
`climon2_client_get_interface`, has no soname and no
[RPATH or RUNPATH](TERMS.md#rpath-and-runpath), and needs no shared library
beyond those of [glibc](TERMS.md#glibc). Release builds check every one of
these rules. Do not use `RTLD_DEEPBIND` or separate `dlmopen` namespaces: each
namespace loads its own glibc, so one release's memory could be freed by
another release's allocator, and glibc allows only a few namespaces.

`LD_LIBRARY_PATH` never redirects an absolute path. On Linux, the absolute
path plus the release identity built into the library is the whole proof of
the exact release. The monitor does not inspect the file before loading it,
because the file could change between the inspection and the load.

Classify Linux load failures from the loader's own checks, never from
`dlerror()` text. A missing file is a missing library. An [ELF](TERMS.md#elf)
header that is not x86-64 is an incompatible architecture. A `dlopen` failure
after both checks pass is a missing dependency, reported with the `dlerror()`
text only for diagnosis. A missing `climon2_client_get_interface` is a missing
symbol. macOS classifies the same way, except that a file without a
[Mach-O slice](TERMS.md#mach-o-and-slices) dyld would load into the running
process is an incompatible architecture. A file the loader cannot open at all,
missing or unreadable, counts as a missing library, and an ELF header it
cannot read as not x86-64 (owner decision on
[#84](https://github.com/jackgeek/climon2/issues/84)).

`LD_PRELOAD`, `LD_AUDIT`, `LD_LIBRARY_PATH` and `GLIBC_TUNABLES` come from the
user's environment and still affect the glibc libraries the client library
uses. They are the user's own configuration: the monitor neither clears them
nor refuses to run because they are set.

Installed libraries are trusted native code. Initializers execute before
identity validation; compatibility checks do not sandbox a malicious artifact.
A native crash during loading can terminate the whole monitor.

## Loading and instance lifetime

Perform loading and validation away from the rendering/input thread. Share one
load attempt per release and one loaded library among that release's instances.
A stalled load must not freeze global monitor actions. It can still prevent
other releases from starting: the system loader serializes loading, and glibc
holds one process-wide lock while a library and its initializers load. A load
that never returns is a native fault, like a crash. There is no load deadline,
because a load cannot be cancelled.

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
The agreed [communication contract](README.md#shared-in-process-communication-contract)
adds a per-instance thread-safe
[communication handle](TERMS.md#communication-handle) for nonblocking submit/drain,
without callbacks into monitor application code. Shutdown is independent of
ordinary queue capacity; connection work remains on the dedicated client thread.

Complete instance and internal-worker cleanup before the client thread returns.
The monitor joins the thread before reporting completed shutdown. Release the
control handle only after its permitted concurrent uses end; exact handle
validity and release operations follow the [native calls](monitor-client/core-v1.md#function-calls-and-ownership). A final
[event](TERMS.md#event), connection closure, or daemon-accepted
[detach](../../GLOSSARY.md#attachment-and-control) is not proof of cleanup or
thread completion. Cleanup failure must be reported, not called success.

Never forcibly terminate a stalled thread or unload running code. The agreed,
unimplemented [lifecycle contract](../monitor-lifecycle.md#coordinated-shutdown)
requests shutdown concurrently for every owned instance, including passive,
attached, partially started and failing ones. Keep draining/releasing deliveries;
required atomic [Leave](daemon-client/request-leave.md) removes registration and
membership even if promotion races with shutdown. Daemon acceptance, closure,
final outcome, worker cleanup and thread join remain distinct evidence.
If cleanup exceeds its eventual budget, attempt
[host](../../GLOSSARY.md#sessions) restoration/root
[attention](../../GLOSSARY.md#status-and-attention)
[clear](monitor-host/report-clear.md), report unconfirmed work, then fail-stop
the whole monitor process. Never
claim a stalled thread joined or a lost Leave response succeeded.
Load/start concurrency, progress deadlines, and admission/exhaustion policies
are specified in [operational limits](../monitor-operational-limits.md).
That agreed, unimplemented specification accounts for thread/worker/descriptor
reservations and retained per-release resources, not just ordinary queue bytes.

## Per-instance inputs and memory

Supply explicit per-instance paths, validated settings snapshots, session
identity, and operation. Copy startup inputs before creation returns. Resolve
relative paths before creation and preserve
[native paths](TERMS.md#native-path) losslessly using length-delimited native
data, not [UTF-8](TERMS.md#unicode-and-utf-8)-only strings.
[Native shared values](monitor-client/core-v1.md#shared-values) specify absolute
Unix bytes and counted text/binary buffers. Create performs only atomic local
setup and returns all three handles or none; Open/Kill are explicit queued work.

Do not treat process-global arguments, environment, settings, current directory,
signals, or [standard streams](TERMS.md#standard-streams) as private instance
inputs. Libraries must not mutate environment, current directory, signal
handlers, or standard streams. Diagnostics are instance-scoped;
the monitor owns its [host](../../GLOSSARY.md#sessions) terminal.

Each allocator frees its own allocations. Every later buffer must explicitly
use copying, call-scoped borrowing, or ownership transfer with a matching release
operation. Never retain a caller buffer by implication or free a library
allocation with another release's allocator. The communication interface does
not call monitor application code.

Submit copies all accepted data before return and retains no rejected arguments.
Drain transfers immutable library-owned deliveries, released through the same
library. Thread-safe release obeys coordinated instance lifetime. Outstanding
deliveries prevent successful destruction; stop new calls, release objects,
and coordinate polling of the borrowed
[notification descriptor](TERMS.md#pollable-notification-descriptor) before
dedicated-thread cleanup completes. Descriptor ownership never transfers.

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
Automatic exponential passive recovery covers recoverable startup failures
(including missing libraries), transport loss and safely contained worker
panics. Permanent protocol/interface rejection and invalid configuration stop
retries with explicit errors. A repaired installation can recover automatically.
Each fresh attempt waits for prior deliveries/handles, workers and instance
thread cleanup/join; no replacement overlaps old running code. Failure to prove
cleanup uses whole-process fail-stop, not another attempt. Keep opened library
handles loaded until process exit. Recovery never requests takeover or replays
indeterminate work; final outcomes stop retry. The
[lifecycle contract](../monitor-lifecycle.md#history-and-passive-recovery) owns
policy; [operational limits](../monitor-operational-limits.md#passive-retry)
supplies configurable delay, cap and jitter.

This is not native-crash isolation. Abort, native faults, and panic-abort builds
can terminate the monitor and every hosted client. No new session-survival
guarantee exists: [ordinary sessions](../../GLOSSARY.md#sessions) retain the
daemon's existing [leave policy](../../GLOSSARY.md#attachment-and-control) and
agreed succession.
[Headless sessions](../../GLOSSARY.md#sessions) explicitly survive zero
clients even after later attachment loss, until completion or
[Kill](../../GLOSSARY.md#attachment-and-control); that lifetime
exception is implemented on the current private protocol and adds no
native-crash isolation.

## Required automated evidence

These are obligations for later implementation, not tests already run:

- Exercise packaged dynamic libraries through the public interface: multiple
  same-release instances and simultaneous different-release libraries.
- Verify exact artifact lookup, identity and revision validation before
  connection, and explicit missing/unsupported/mismatched artifact failures.
- Use different private dependency behavior in mixed-release fixtures to detect
  accidental symbol/dependency cross-binding.
- Cover native paths, independent paths/settings/diagnostics/resources, and
  continued progress when another instance stalls or fails, or another release
  fails to load.
- Check copying, borrowing and transferred-buffer lifetimes, allocator pairing,
  internal-worker cleanup, control-handle lifetime and actual thread joins,
  including partial startup and recoverable panics.
- Cover concurrent shutdown and promotion racing with Leave, lost response
  versus closure/outcome/join, outstanding deliveries and stalled workers/
  threads with bounded whole-process failure, plus automatic passive recovery
  after safe failures and no overlapping replacement instances.
- Exercise native crash/abort in a disposable monitor process. Report
  whole-monitor failure exposure; do not assert process-isolated client
  survival or guaranteed session survival.
- Produce all of this evidence on x86-64 GNU/Linux and on both macOS
  architectures. On Linux, also check that every release build's library
  exports one symbol, has no soname, RPATH or RUNPATH, and depends only on
  glibc.

The agreed [communication decision](https://github.com/jackgeek/climon2/issues/66)
owns [operation delivery and representation, ordering and correlation](README.md#shared-in-process-communication-contract).
The [agreed lifecycle](../monitor-lifecycle.md) owns recovery and incomplete
cleanup; operational limits own budgets;
interface schemas own exact declarations and valid call examples. These
decisions must preserve this embedding contract without inventing
[readiness](../../GLOSSARY.md#protocol-and-plugin-concepts) or successful cleanup
from library loading.
