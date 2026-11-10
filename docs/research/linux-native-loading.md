# Linux native client-library loading: facts and open choices

Research for [#131](https://github.com/jackgeek/climon2/issues/131), part of
[#124](https://github.com/jackgeek/climon2/issues/124); it informs
[#125](https://github.com/jackgeek/climon2/issues/125). Baseline: `origin/main`
`153dd5c`. This note records facts and options only. It selects no loading policy,
amends no specification and implements nothing. glibc source is pinned to tag
`glibc-2.39` (Ubuntu 24.04 ships `2.39-0ubuntu8.x`,
[packages.ubuntu.com/noble/libc6](https://packages.ubuntu.com/noble/libc6)). The Arch
development host has a newer glibc (`ldd` reports 2.44), so behaviour that matters
should be re-confirmed on both. Man-page links are current upstream man7 text.

Labels: **[Current]** implemented on `main`; **[Plan]** agreed, unimplemented
monitor plan; **[Linux]** external fact; **[Choice]** needs a human decision.
Plain "inferred" marks anything not verified against a real artifact.

## 1. Where things stand

- **[Current]** No loader, cdylib, C ABI or notification descriptor exists. Platform
  seams are `compile_error!` off macOS, for example
  `crates/climon2-platform/src/peer.rs:2-3` and `runtime.rs:2-3`.
- **[Plan]** The monitor loads
  `<executable home>/<session version>/libclimon2_client.dylib` by absolute path with
  eager, local resolution, validates identity before any connection, retains every
  opened handle until exit and never unloads
  (`docs/protocol/library-embedding.md:44-62,70-79`;
  [#73](https://github.com/jackgeek/climon2/issues/73) body, user stories 131-133, and
  its comment, "Client supervisor → Loading"). Other platforms "choose their filenames
  when supported" (`library-embedding.md:48-49`). #73's "Platforms" out-of-scope
  bullet (monitor support on Linux) is superseded by #124's destination.
- **[Plan]** One validated loader lives in `climon2-platform`, no third-party loader
  crate ([#84](https://github.com/jackgeek/climon2/issues/84)); `#[repr(C)]`
  declarations live in `climon2-core` ([#75](https://github.com/jackgeek/climon2/issues/75)).
  Both are written for `.dylib`/macOS.
- **[Plan]** Revision 1 "targets the supported macOS arm64 and x86_64 platform C
  calling conventions" (`docs/protocol/monitor-client/core-v1.md:18-19`).
  `notification` is a "borrowed nonnegative macOS pollable descriptor"
  (`core-v1.md:225`); the header type is `int32_t *`
  (`docs/protocol/monitor-client/client-v1.h:457`). The descriptor kind is
  deliberately not frozen (`docs/protocol/README.md:416-424`).
- **[Plan]** Failure model: trusted code, initializers run before identity validation,
  a native crash can end the whole monitor, no new isolation or session-survival
  promise (`library-embedding.md:59-62,172-179`; #124 "Out of scope"). Nothing
  below changes this.

## 2. Exact-path identity

- **[Linux]** A `dlopen` path containing `/` is used as a pathname and bypasses the
  search order ([dlopen(3)](https://man7.org/linux/man-pages/man3/dlopen.3.html)); glibc
  skips directory search when the name has a slash
  ([`elf/dl-load.c:2017`](https://raw.githubusercontent.com/bminor/glibc/glibc-2.39/elf/dl-load.c)).
- **[Linux]** Before opening, `_dl_map_object` scans the target namespace for a
  loaded object whose recorded names match the request, or whose `DT_SONAME` equals
  it (`dl-load.c:1959-1983`). After opening, it matches by file identity
  (`l_file_id`, device and inode), reuses that object and records the extra name
  (`dl-load.c:988-1000`). dlopen(3): reopening the same object returns the same handle.
- **Consequence.** Identity is the file and namespace, not the path string. A
  symlink such as `current`, a hard link or a bind mount to an already-loaded file
  returns the existing handle; a different release's file is a distinct object.
  `dlopen` itself never checks that the file is the intended release. Only the
  post-load description check does, after initializers have run.
- **[Linux]** `dlinfo(RTLD_DI_LINKMAP)` exposes `l_name` and `RTLD_DI_ORIGIN` the
  directory ([dlinfo(3)](https://man7.org/linux/man-pages/man3/dlinfo.3.html)); `l_name`
  is the name given, not a canonical path.
- **[Linux]** If a `dlopen` fails after mapping, glibc itself unloads the object
  (`elf/dl-open.c:937`, `_dl_close_worker (args.map, true)`). The plan retains only
  successfully opened handles, so there is no conflict.
- **TOCTOU.** `dlopen` takes a pathname, not a descriptor (no `fdlopen` in
  dlopen(3)), so any check before the call (realpath, owner, mode) is racy. See C1.
- **[Linux]** A loaded object's `DT_SONAME` can satisfy later requests by that bare
  name. rustc only passes `-soname` where its linker code asks
  (`compiler/rustc_codegen_ssa/src/back/linker.rs:480-482`, 1.96.0); whether this
  project sets one is not established.
- **[Linux]** Ambient process inputs matter: `LD_BIND_NOW`, `LD_PRELOAD`,
  `LD_LIBRARY_PATH`, `LD_AUDIT`, and `GLIBC_TUNABLES`
  ([ld.so(8)](https://man7.org/linux/man-pages/man8/ld.so.8.html)); secure-execution
  mode (setuid) ignores some of them, which an ordinary user process does not use.

## 3. Eager binding, scope and interposition

- **[Linux]** `RTLD_NOW` resolves every undefined symbol before `dlopen` returns or
  fails; `RTLD_LOCAL` (default) does not make the library's symbols available to later
  loads (dlopen(3)). This is the analogue of the plan's "eager and local".
- **[Linux]** Resolution order for the library's references: main program and its
  startup dependencies, then `RTLD_GLOBAL` objects, then the library and its own
  dependencies (dlopen(3); same breadth-first idea in
  [gABI ch. 5](https://refspecs.linuxfoundation.org/elf/gabi4+/ch5.dynamic.html)).
  **A global definition beats the library's own** unless the library binds
  symbolically, hides the symbol, or is opened with `RTLD_DEEPBIND`. binutils:
  `-Bsymbolic` binds to the library's own definition, whereas normally "a program
  linked against a shared library [can] override" it
  ([ld Options](https://sourceware.org/binutils/docs/ld/Options.html)). `RTLD_DEEPBIND`
  puts the library's scope first (dlopen(3), since glibc 2.3.4). Its side effects on
  allocator and libc symbols were not verified.
- **[Linux]** A Rust cdylib is linked with a version script exporting only listed
  symbols as `global:` and `local: *;` (`linker.rs:855-900`), and x86_64 linux-gnu
  uses full RELRO, `-z relro -z now` (`rustc_target/src/spec/base/linux.rs:12`,
  `linker.rs:665-667`). So the exported set and eager binding follow from the build.
  Inferred from rustc 1.96.0 source, not from a built artifact.
- **Cross-release hazard.** `DT_NEEDED` lookup is namespace-wide and name-based: a
  second release asking for `libX.so.1` is satisfied by an already-loaded object
  named or soname'd `libX.so.1` (`dl-load.c:1959-1983`), regardless of `RTLD_LOCAL` and
  of the second library's `RUNPATH`. Same-named private dependencies in two release
  folders can therefore cross-bind silently. dlopen(3) also notes an `RTLD_LOCAL`
  dependency can be promoted to global through another `RTLD_GLOBAL` object.
  Options: bundle no private shared dependencies (a Rust cdylib statically links Rust
  crates, so its needs should be system libraries only; confirm on the real
  artifact), use release-unique sonames, or separate namespaces (section 4).
- **Private dependency search.** For non-slash `DT_NEEDED` names: `RPATH` (only if no
  `RUNPATH`), then `LD_LIBRARY_PATH`, then `RUNPATH`, then the ld.so cache, then
  `/lib` and `/usr/lib`. `RUNPATH` serves only that object's direct dependencies;
  `$ORIGIN` expands to its directory
  ([ld.so(8)](https://man7.org/linux/man-pages/man8/ld.so.8.html), "dependency"
  section and `$ORIGIN`). **`LD_LIBRARY_PATH` outranks `RUNPATH`**, so an ambient
  variable can redirect a "release-private" dependency. `DT_RPATH` outranks it but is
  deprecated and applies to children. This is the key difference from the plan's
  "resolve relative to the library, not ambient search paths"
  (`library-embedding.md:52-58`).
- **[Linux]** `dlsym(handle, …)` searches the handle's object and its dependency tree
  ([dlsym(3)](https://man7.org/linux/man-pages/man3/dlsym.3.html)), so
  `climon2_client_get_interface` resolves in the intended library. `dlerror()` is
  MT-Safe ([dlerror(3)](https://man7.org/linux/man-pages/man3/dlerror.3.html)).
- **Allocator pairing (inferred).** A Rust cdylib's default allocator calls
  `malloc`/`free`, resolved through global scope to the process libc or any
  interposed allocator. Releases then share one allocator in practice, but not by
  guarantee, so "each allocator frees its own allocations" stays necessary.

## 4. Namespaces and modes

- **[Linux]** `dlmopen(LM_ID_NEWLM, …)` loads into a fresh link-map list; resolution
  is confined to objects loaded there and the object must itself reference all
  dependencies (dlopen(3)). glibc supports 16 namespaces (`DL_NNS 16`,
  `sysdeps/generic/ldsodefs.h:311`); `glibc.rtld.nns` defaults to 4
  (`elf/dl-tunables.list:128-133`), set only via `GLIBC_TUNABLES` at process start.
  Exhaustion errors "no more namespaces available for dlmopen()" (`elf/dl-open.c:866`).
- **[Linux]** dlopen(3) BUGS: `RTLD_GLOBAL` with `dlmopen` errors, and `RTLD_GLOBAL`
  `dlopen` from a non-base namespace crashes. A new namespace loads its own libc
  (`libc_already_loaded`, `dl-open.c:566,781`), so allocator, stdio, TLS and thread
  state are per namespace. Behaviour of Rust threads, thread-locals and signal state
  in a secondary namespace was not verified.

| Mode | Gains | Costs and risks |
|---|---|---|
| `dlopen(abs, RTLD_NOW\|RTLD_LOCAL)`, base namespace (plan analogue) | Matches plan; one libc/allocator | `DT_NEEDED` name collisions; global definitions can interpose; `LD_LIBRARY_PATH` precedes `RUNPATH` |
| plus `RTLD_DEEPBIND` | Library prefers its own definitions | glibc/Solaris extension; effects on libc/allocator symbols unverified |
| plus `RTLD_NODELETE` | Pins even if `dlclose` is called by mistake | Redundant with plan; no cross-binding effect |
| `dlmopen(LM_ID_NEWLM)` per release | Strongest symbol/dependency separation | At most 16 releases and a default of 4; libc per release; documented bugs; all deps linked explicitly |
| Static private deps, system libraries only | Removes name-collision risk | Build rule to prove via the artifact's `DT_NEEDED` |

## 5. Initializers, locks and lifetime

- **[Linux]** Constructors and `.init_array` run before `dlopen` returns (dlopen(3)).
  glibc runs them inside `_dl_open`, which holds the process-wide recursive
  `dl_load_lock` (`elf/dl-open.c:851` lock; `call_dl_init` at 526-531, called from
  `dl_open_worker_begin`; unlock 945/956). **A stalled constructor holds the lock and
  blocks every other `dlopen`.** `__cxa_thread_atexit_impl` also takes the same lock
  when a thread registers a TLS destructor (`stdlib/cxa_thread_atexit_impl.c:117`), and
  `dlclose` does (`elf/dl-close.c:763-795`). The plan's "a stalled load must not ...
  indefinitely prevent other releases from starting" (`library-embedding.md:64-68`)
  therefore cannot hold for loads on glibc, and `scheduling.loads` concurrency is moot.
  Load workers still protect the UI thread. C4.
- **[Linux]** Rust's `std` places an `.init_array` function in glibc cdylibs to capture
  argc/argv (`library/std/src/sys/args/unix.rs:116-134`), an example of an initializer
  running before identity validation. `sys/pal/unix/mod.rs:23-39` notes runtime
  init, including `SIGPIPE` handling, is "not guaranteed to run ... when Rust code is
  called externally", so a cdylib inherits the host disposition. Per-call
  `MSG_NOSIGNAL` ([send(2)](https://man7.org/linux/man-pages/man2/send.2.html)) is the
  way to avoid depending on it; whether `std`'s `UnixStream` does this was not checked.
- **[Linux]** `dlclose` does not unload while references or pending TLS destructors
  remain (`dl-close.c:179-182`); `-z nodelete` and `RTLD_NODELETE` prevent unloading
  (dlopen(3); ld Options). The plan never calls `dlclose`, so no unload hazard arises.
  Order of exit-time `.fini_array` against threads still running library code was not
  analysed; whole-process fail-stop governs stalls (`library-embedding.md:98-111`).
- **[Linux]** Thread stacks are caller-chosen; glibc caches freed stacks up to 40 MiB
  (`nptl/nptl-stack.c:23`), so joined threads may not return memory immediately, which
  matters to resident-memory accounting (`docs/monitor-operational-limits.md:113-118`).
  Static TLS is bounded by surplus reserved at startup (`elf/dl-tls.c:96-111`,
  `glibc.rtld.optional_static_tls` default 512, `dl-tunables.list:134-138`); whether
  the real cdylib needs static TLS is unknown until built.
- **[Linux]** Loader failures surface as `dlerror()` strings, not typed codes. #84's
  per-case errors (missing dependency, wrong architecture, missing symbol) need string
  classification, an ELF header pre-check, or both. C6.

## 6. Notification descriptors and C ABI layout

### 6.1 C ABI

- **[Linux]** The x86-64 psABI gives LP64: `int` 4, `long` and pointers 8, natural
  alignment, struct size a multiple of alignment, padding undefined
  ([psABI `low-level-sys-info.tex:33-83,167-185`](https://github.com/hjl-tools/x86-psABI/blob/master/low-level-sys-info.tex),
  the maintainer's LaTeX source; the GitLab copy returned HTTP 403). Revision 1 uses
  fixed-width integers and pointers only, so layouts match LP64 macOS.
- **Proof run.** `gcc -fsyntax-only -Wall -I. layout.c`, `gcc … examples.c` and
  `clang -fsyntax-only -I. layout.c` in `docs/protocol/monitor-client/` all succeed on
  this x86_64 Linux host, so the `_Static_assert`s in `layout.c:5-18` hold. This shows
  C layout only, not Rust `#[repr(C)]` conformance or runtime behaviour.
- **Return convention.** Every table call returns `cm1_call` by value
  (`core-v1.md:204`, `client-v1.h:158-164`): 4+4 plus `cm1_error` (4+4+16+16) is 48
  bytes. A struct larger than eight eightbytes is class MEMORY, so the caller passes
  hidden storage in `%rdi` and `%rax` returns it (psABI `low-level-sys-info.tex:455-466,625-640`).
  Rust `extern "C"` and C agree per the platform ABI, but Linux x86_64 is not in
  the spec's "targets" sentence (`core-v1.md:18-19`), and the layout test is planned
  for macOS runners only. C5.
- **[Linux]** An unwinding `extern "C"` function aborts under Rust 1.96
  ([Reference, functions](https://doc.rust-lang.org/reference/items/functions.html)),
  consistent with the plan's panic catching. A cdylib builds as `*.so` on Linux
  ([Reference, linkage](https://doc.rust-lang.org/reference/linkage.html)).

### 6.2 Notification descriptor

The ABI fixes only a nonnegative borrowed `int32_t`, polled and never read, written or
closed, with signalling that "cannot miss work or closure".

- **[Linux]** `eventfd(2)`: an 8-byte counter, readable while nonzero, maximum
  `0xfffffffffffffffe`, `EFD_CLOEXEC`/`EFD_NONBLOCK` available
  ([eventfd(2)](https://man7.org/linux/man-pages/man2/eventfd.2.html)); one descriptor;
  Linux-only. `pipe2(2)` with `O_CLOEXEC|O_NONBLOCK`
  ([pipe(2)](https://man7.org/linux/man-pages/man2/pipe.2.html)) is portable but costs two
  descriptors against `resources.descriptors_per_client` = 8 and headroom
  (`monitor-operational-limits.md:115-117`). C7.
- **[Linux]** `O_CLOEXEC` at creation is "essential in some multithreaded programs"
  to avoid leaking to a concurrent `fork`+`execve`
  ([open(2)](https://man7.org/linux/man-pages/man2/open.2.html)); later `fcntl` is racy.
- **[Linux]** `poll(2)` is level-triggered, unaffected by `O_NONBLOCK`, with `nfds`
  bounded by `RLIMIT_NOFILE` ([poll(2)](https://man7.org/linux/man-pages/man2/poll.2.html));
  `epoll(7)` supports edge triggering and has duplicate-descriptor caveats
  ([epoll(7)](https://man7.org/linux/man-pages/man7/epoll.7.html)). Count-free,
  drain-based readiness suits level semantics; `select`'s `FD_SETSIZE` is unsuitable.
- **[Linux]** Closing a descriptor another thread is polling is unspecified; on Linux
  `select()` keeps waiting
  ([select(2), "Multithreaded applications"](https://man7.org/linux/man-pages/man2/select.2.html);
  poll(2) NOTES defer to it), and the number can be reused. This makes the plan's
  "borrowed until destroy, stop polling first" rule (`library-embedding.md:139-146`)
  a hard Linux requirement, not style.
- **[Current]** The code already uses `libc::poll`, `O_NONBLOCK` and `UnixStream`
  (`crates/climon2-client/src/transport.rs:37-47`, `bridge.rs:225-248`; daemon
  `pty.rs:97-104,291-292`); none is Linux-qualified yet.

### 6.3 Unix communication primitives

- **[Current]** Peer credentials use `getpeereid` under `cfg(target_os = "macos")`
  (`peer.rs:24-31`). [unix(7)](https://man7.org/linux/man-pages/man7/unix.7.html)
  documents `SO_PEERCRED` (`struct ucred`, captured at `connect`/`listen`/`socketpair`);
  man7 has no `getpeereid(3)` page (404) and glibc export was not established.
- **[Linux]** `sun_path` is 108 bytes on Linux (unix(7)); ADR 0003 keeps
  `/tmp/climon2-<uid>-<hash8>` (`docs/adr/0003-platform-and-shell-seams.md:31`). #126
  owns peer-credential and path-length facts.
- The portable daemon wire and cross-release contract are unaffected by section 6.

## 7. Choices needing a human decision

- **C1. Exact-path policy.** Plain absolute-path `dlopen`, or extra pre-checks
  (realpath, reject symlinks, owner and mode, device/inode) with the TOCTOU gap accepted.
- **C2. Mode and namespace.** Base `RTLD_NOW|RTLD_LOCAL` versus `DEEPBIND` or `dlmopen`.
- **C3. Private-dependency rule.** Forbid private shared dependencies, require
  release-unique sonames, or rely on namespaces; whether the build sets a soname or
  `RUNPATH`.
- **C4. Load stall bound.** glibc serialises `dlopen`, constructors and TLS-destructor
  registration under one lock: is the eventual whole-process fail-stop enough, or does
  load need its own deadline; how is `scheduling.loads` described.
- **C5. Specification amendments** (owned by #125): Linux filename, the "platform C
  calling conventions" sentence, the "macOS" descriptor wording, layout-test platforms,
  `docs/runtime-artifacts.md` (`.dylib` on macOS), #84's error list.
- **C6. Error classification** from `dlerror()` text and ELF header checks.
- **C7. Descriptor kind** (`eventfd` or pipe), always close-on-exec, and its count in
  `cm1_requirements`.
- **C8. Ambient environment.** Sanitise or only document `LD_LIBRARY_PATH`,
  `LD_PRELOAD`, `LD_BIND_NOW`, `LD_AUDIT` and `GLIBC_TUNABLES`.

## 8. Gaps

- No cdylib was built and no `dlopen` or multi-release experiment was run, per scope.
  Claims about `DT_NEEDED`, soname, static TLS, exports and `RUNPATH` are inferred
  from rustc 1.96.0 source and need confirmation once #75 produces an artifact.
- glibc was read at 2.39; the dev host differs (2.44).
- Not analysed: `DEEPBIND` side effects, Rust threads in secondary namespaces,
  exit-time destructor ordering, `std` `MSG_NOSIGNAL` use, `getpeereid` in glibc.
- Not read in full: `monitor-lifecycle.md`, `monitor-acceptance.md` beyond search hits,
  ADRs 0001/0006 beyond headers. The `feature/linux-*` branches were not read or touched.
