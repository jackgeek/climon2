# Linux quality tooling and release portability

Research for [#127](https://github.com/jackgeek/climon2/issues/127), child of
[#124](https://github.com/jackgeek/climon2/issues/124). It informs #128 (quality and
packaged qualification), #129 (platform support and lifecycle) and #130 (packaging and
upgrade). It states facts and options only. It approves no exemption, native-source
filter, gate design or release policy.

Baseline: `origin/main` at `153dd5c`. Investigated on the Arch-based development host
(glibc 2.44, Python 3.14.8, Rust `1.96.0`, whose rustc uses LLVM 22.1.2).

Labels: **[V]** read directly from a pinned source or a local artifact; **[D]** derived
from cited sources, not executed; **[X]** exploratory-branch evidence, never acceptance
evidence. Nothing was installed, built, mutated or modified. The exploratory worktrees
(`linux`, `ldist`, `ltest`) were read only.

## 1. Repository starting point (`origin/main`)

- **[V]** Pins: crapper `6fd92c4c…`, mutator `a5a0cb80…`, cargo-llvm-cov `0.9.1`, CRAP
  threshold `5`, two mutation workers, shipped source is eight `crates/*/src`
  directories (`xtask/src/quality.rs:11-27`).
- **[V]** Coverage runs cargo-llvm-cov with `LLVM_PROFILE_FILE=…/climon2-%c%m.profraw`.
  The only platform flag is the Darwin `-Wl,-sectalign,__DATA,<section>,0x4000` for
  `__llvm_prf_{cnts,data,bits}` (`xtask/src/coverage.rs:15-16,49-56,65`).
- **[V]** `tooling-probe` proves an installer-built binary contributes coverage: one
  normal run plus one exec-replaced run gives exactly two `staged_probe` hits
  (`xtask/src/quality.rs:340-390`). `mutation-probe` proves baseline plus six mutant
  variants each ran their own binary under a forbidden shared target dir
  (`xtask/src/quality.rs:237-262`). Neither has a dedicated SIGKILL case.
- **[V]** CI: `quality` and `dist` run on `macos-15` only; `archive` runs on `macos-15`
  and `macos-15-intel`; Ubuntu runs only `climon2-core` and launcher tests (`neutral`)
  (`.github/workflows/check.yml:44-60,60-69,103-105,121-133`). Release publishes from
  `macos-15` with `actions/attest-build-provenance@v2`, one `macos-universal` archive,
  `install.sh` and `SHA256SUMS` (`.github/workflows/release.yml:14-42`).
- **[V]** `xtask dist` requires arm64 macOS, then `lipo`, `codesign`, `tar -czf` and
  `shasum -a 256` (`xtask/src/dist.rs:4-9,33-62`). `install.sh` hard-codes
  `macos-universal` and `shasum`.
- **[V]** Every platform seam is `#[cfg(target_os = "macos")]` with `compile_error!`
  otherwise (`crates/climon2-platform/src/{connection,daemon,peer,presentation,process,pty,runtime,terminal}.rs`).
  ADR 0003 says Ubuntu 24.04 must be addable without a redesign and keeps the Linux
  runtime dir under `/tmp` with systemd-tmpfiles sweep risk
  (`docs/adr/0003-platform-and-shell-seams.md:3,31,45`).
- **[V]** The remote has **no releases and no tags** (`gh api repos/jackgeek/climon2/releases`
  and `…/tags` return `[]`). The repository is public.

## 2. LLVM coverage on Linux: normal, exec-replaced and killed processes

### 2.1 Upstream documentation

- **[V]** `%c` keeps coverage if the program "crashes, or is killed by a signal", but "is
  only supported on Darwin at the moment. Support for Linux may be mostly complete but
  requires testing"
  ([`SourceBasedCodeCoverage.rst` L102-110 @ llvmorg-22.1.2](https://github.com/llvm/llvm-project/blob/llvmorg-22.1.2/clang/docs/SourceBasedCodeCoverage.rst#L102-L110)).
  The rustc book says "on some platforms, including Darwin"
  ([`instrument-coverage.md` L116 @ 1.96.0](https://github.com/rust-lang/rust/blob/1.96.0/src/doc/rustc/src/instrument-coverage.md#L116)).
- **[V]** The same doc describes a counter-relocation alternative that "can be also
  enabled for other platforms by passing the `-runtime-counter-relocation` option", at
  the cost of binary size, speed and duplicated counters (same file, L116-137).

### 2.2 Runtime behaviour on ELF

Cites [`compiler-rt/lib/profile/InstrProfilingFile.c` @ llvmorg-22.1.2](https://github.com/llvm/llvm-project/blob/llvmorg-22.1.2/compiler-rt/lib/profile/InstrProfilingFile.c).

- **[V]** The Darwin branch (L105-203) requires page-aligned counter, bitmap and data
  sections, which is why `coverage.rs` passes `-sectalign`.
- **[V]** The ELF branch (from L203) sets `ContinuousModeSupported = 1` and
  `UseBiasVar = 1`: it maps the profile file `MAP_SHARED` and points counters at the
  mapping through `__llvm_profile_counter_bias`. No section alignment is needed.
- **[V]** If the binary was not compiled with relocation, init reports "Neither
  `__llvm_profile_counter_bias` nor `__llvm_profile_bitmap_bias` is defined" and returns
  (L628-640). Continuous mode on ELF thus silently does nothing without
  `-runtime-counter-relocation` (a runtime stderr message, not a build failure).
- **[V]** The compiler honours the option only off Mach-O (returns false for Mach-O,
  reads the `cl::opt` elsewhere, default-on only for Fuchsia:
  [`InstrProfiling.cpp` L91-94,844-851](https://github.com/llvm/llvm-project/blob/llvmorg-22.1.2/llvm/lib/Transforms/Instrumentation/InstrProfiling.cpp#L844-L851)).
  So the two OS families need different, non-interchangeable mechanisms.
- **[V]** rustc passes it as `-Cllvm-args=-runtime-counter-relocation`
  ([`tests/codegen-llvm/pgo-counter-bias.rs` L3-4 @ 1.96.0](https://github.com/rust-lang/rust/blob/1.96.0/tests/codegen-llvm/pgo-counter-bias.rs#L3-L4);
  the test notes the bias symbol may be internalized under LTO).
- **[V]** With `%m`, continuous mode opens, locks and maps the shared file; increments
  are visible to every process mapping it (L653-683). The lock is released after init.
  The file is keyed by binary signature, so a re-exec of the same binary shares one file
  and different binaries get different files. Without `%m` the ELF path has a TODO that
  a second DSO truncates the first's file (L240-246). `%c%m` takes the merge path.
- **[D]** Exec and SIGKILL: the mapping is shared with the file, so increments need no
  exit handler; exec drops the mapping but not the file; SIGKILL skips `atexit`. This
  follows from the code and `%c` docs but has **not** been executed on Linux here.

### 2.3 cargo-llvm-cov 0.9.1

Cites [`taiki-e/cargo-llvm-cov` v0.9.1](https://github.com/taiki-e/cargo-llvm-cov/tree/v0.9.1).

- **[V]** It defaults `LLVM_PROFILE_FILE` to `<name>-%p-%m.profraw` and applies
  instrumentation through a `RUSTC_WRAPPER` (`src/main.rs:207-246`; `src/wrapper.rs:24-31`),
  and adds `--cfg=coverage` (`src/main.rs:194-199`).
- **[D]** `xtask`'s extra `RUSTFLAGS` is additive to wrapper flags. Whether
  `-Cllvm-args=-runtime-counter-relocation` in it takes effect on every crate (including
  dependencies and build scripts) is **unproven**.

## 3. `cfg`-inactive code vs compiled-but-uncovered code

### 3.1 Compilation side

- **[V]** A false `cfg` predicate removes the form from the source
  ([Rust Reference, conditional compilation](https://github.com/rust-lang/reference/blob/master/src/conditional-compilation.md)).
  rustc instruments functions in compiled code, including unused ones
  ([`instrument-coverage.md` L54 @ 1.96.0](https://github.com/rust-lang/rust/blob/1.96.0/src/doc/rustc/src/instrument-coverage.md#L54)).
- **[D]** So a cfg-inactive function has no coverage-mapping records at all, unlike a
  compiled function no test ran (`DA:<line>,0`). A `#[path]` module under an inactive
  `cfg` is not loaded. Not executed here; needs a one-file probe.

### 3.2 crapper (`6fd92c4c`) cannot distinguish them

Source: [`unclebob/crapper@6fd92c4c`](https://github.com/unclebob/crapper/tree/6fd92c4c9bcc56efb64a2f8743d5a2993bcfd42d).

- **[V]** Rust analysis is tree-sitter only, with **no `cfg` evaluation**. Every
  `function_item` with a body is scored except nested functions and anything under a
  module named `tests`; `#[cfg(test)]` is not recognised. Complexity counts `if`, loops,
  each `match` arm, `?` and `&&`/`||` (`src/crapper/languages/rust.py:1-8,26-43,157-190`).
- **[V]** Discovery walks directories skipping only `target`/test dirs and test files
  (`src/crapper/discover.py:93-108`), so the other OS's out-of-line files are scored.
- **[V]** A file absent from LCOV gives `None`, which `analyze.py:12-25` turns into
  0.0%. For a present file, only lines that have `DA` records count; total 0 gives 0.0
  (`coverage.py:205-218`). Branch records win when present (`coverage.py:442-469`).
- **[V]** `CRAP = CC²·(1-cov)³ + CC` (`src/crapper/crap.py:6-17`). At 0%: CC 1 gives 2,
  CC 2 gives 6, CC 3 gives 12; against threshold 5 only CC ≥ 2 fails (the repo's probes
  use these, `xtask/src/quality.rs:409-450`).
- **[D]** Consequences:
  1. A whole cfg-inactive function with CC ≥ 2 scores above 5 on the other OS; CC 1
     passes even at 0%.
  2. A cfg-inactive block inside an active function has no coverage lines but still adds
     its decision points to CC; coverage is computed over compiled lines only.
  3. A compiled function that never ran shows `DA:…,0` and fails when CC ≥ 2. Crapper
     cannot tell this from case 1; any distinction must come from outside crapper
     (for example the cfg-active file/function set).

### 3.3 mutator (`a5a0cb80`) with existing coverage

Source: [`unclebob/mutator@a5a0cb80`](https://github.com/unclebob/mutator/tree/a5a0cb803b66e0cdb8bf471b3b08af701d424a41).

- **[V]** Sites use the same cfg-blind tree-sitter functions. A site runs only if its
  line has an LCOV hit; otherwise it is "uncovered and is not run", and a file absent
  from coverage warns "its sites are uncovered" (`src/mutator/engine.py:53-58,165-172`;
  `src/mutator/cli.py:133`).
- **[V]** Exit 3 is raised only for survived mutants; uncovered sites never fail
  (`cli.py:121-125,516-526`; `engine.py:120-131`).
- **[D]** "Zero survivors" therefore does not show compiled code is tested. Today only
  the CRAP threshold catches compiled-but-uncovered code, and only for CC ≥ 2.
- **[V]** `xtask` runs mutator over changed or shipped `.rs` files regardless of OS, with
  `cargo test --workspace --locked` as the test command (`quality.rs:27,160-230`).

### 3.4 Python tooling on Linux

- **[V]** Both tools are pure Python with no OS-specific code. The repo pins
  `tree-sitter==0.26.0` and `tree-sitter-language-pack==1.22.1`
  (`tools/quality-requirements.txt`); PyPI publishes `manylinux_2_34_x86_64` and
  cp311-cp314 `manylinux_2_28_x86_64` wheels (PyPI JSON API), within Ubuntu 24.04's
  glibc 2.39 and Python 3.12.3 (runner image). Workers are symlink overlays under
  `target/mutation-workers` sized by `os.cpu_count()` (`mutator/workers.py:25-34,78-93`).

## 4. Isolated worker proofs on Linux

- **[V]** The probes in section 1 name no OS, but depend on the Darwin-only coverage flag,
  the installer (macOS-only seam in `climon2-install`) and installer-built instrumented
  binaries. They can run on Linux only after the coverage flag and installer seam exist.
  Whether they pass on Linux is **unproven**.

## 5. Ubuntu 24.04 x86_64 / glibc baseline vs Arch development

- **[V]** Ubuntu 24.04 ships glibc `2.39-0ubuntu8.x`
  ([Launchpad glibc sources, noble](https://launchpad.net/ubuntu/+source/glibc)). The
  `ubuntu-24.04` runner is 24.04.5, kernel 7.0.0-1017-azure, bash 5.2.21, Python 3.12.3,
  shellcheck 0.9.0, tar 1.35, Rust 1.99.1 preinstalled
  ([Ubuntu2404-Readme](https://github.com/actions/runner-images/blob/main/images/ubuntu/Ubuntu2404-Readme.md)).
  The dev host has glibc 2.44.
- **[V]** The loader refuses a program needing a symbol version the installed glibc lacks
  ("version `%s' not found (required by %s)",
  [`elf/dl-version.c` L135-144 @ glibc-2.39](https://sourceware.org/git/?p=glibc.git;a=blob;f=elf/dl-version.c;hb=refs/tags/glibc-2.39)).
  A binary runs on the build glibc or newer, not older.
- **[V]** Rust's floor for `x86_64-unknown-linux-gnu` is kernel 3.2+, glibc 2.17+
  ([platform-support.md L42 @ 1.96.0](https://github.com/rust-lang/rust/blob/1.96.0/src/doc/rustc/src/platform-support.md#L42));
  that is the toolchain floor, not what a given build links.
- **[V, local artifact]** The Arch-built exploratory archive
  `ldist/target/dist/climon2-0.1.0-linux-x86_64-gnu.tar.gz` needs at most (string scan of
  `GLIBC_2.x` version names, not a verneed parse): `climon2-cli` 2.39, `climon2-daemon`
  2.39, others 2.34. This equals the Ubuntu baseline by coincidence of current code, not
  by guarantee: nothing stops a later change raising it, and Arch gives no build error.
  The symbol requiring 2.39 was not identified.
- **[V]** The runner README lists no `vim`; the packaged tests hard-code `/usr/bin/vim`
  (`crates/climon2-e2e/tests/packaged.rs:269`), so a Linux job must apt-install it. The
  `controlled-bash` action is Homebrew-only (`.github/actions/controlled-bash/action.yml`).
- **[D]** Options (none selected): build releases only on `ubuntu-24.04`; and/or add a
  maximum-`GLIBC_*` assertion (≤ 2.39) over produced binaries; treat Arch builds as
  development evidence only (exploratory `ldist:docs/distribution.md:10-16` takes that
  position as a claim, not a tested gate).

## 6. Hosted CI, artifacts and release mechanics

- **[V]** Public repos get free standard runners: `ubuntu-24.04` x64 (4 CPU/16 GB),
  `macos-15` arm64, `macos-15-intel`
  ([github/docs supported-github-runners](https://github.com/github/docs/blob/main/data/reusables/actions/supported-github-runners.md)).
  runner-images announces `ubuntu-latest` moves to 26.04 in November 2026, so jobs must
  name `ubuntu-24.04` (as `check.yml` `neutral` already does).
- **[V]** Linux runners have passwordless `sudo`
  ([github-hosted-runners](https://github.com/github/docs/blob/main/content/actions/reference/runners/github-hosted-runners.md)),
  so apt installs of test prerequisites are possible. PTY availability for the real
  terminal tests on a hosted runner is **not documented** and unproven; no exploratory
  branch has run in hosted CI.
- **[V]** `upload-artifact` loses file permissions for loose files and recommends
  tarring ([README "Permission Loss"](https://github.com/actions/upload-artifact/blob/main/README.md)).
  Release archives are already `.tar.gz`; `install.sh` loses its bit and is run via bash.
  `download-artifact` supports `pattern` and `merge-multiple`.
- **[V]** GitHub's attestation docs now show `actions/attest` (v4) with `id-token`,
  `contents: read`, `attestations: write`
  ([docs](https://docs.github.com/en/actions/how-tos/secure-your-work/use-artifact-attestations/use-artifact-attestations));
  the repo uses `attest-build-provenance@v2` (`release.yml:29`). Equivalence not checked.
- **[V, local]** `dist.rs` uses plain `tar -czf` (`:47-50`); the exploratory archive has
  uid/gid 1000, build-time mtimes and directory-order members (gzip header mtime 0), so
  rebuilds differ. Reproducible-tar guidance: `--sort=name`, `--mtime`,
  `--owner=0 --group=0 --numeric-owner`, `--format=gnu`
  ([reproducible-builds.org](https://reproducible-builds.org/docs/archives/)); these are
  GNU tar options, while macOS bsdtar differs (inference). No repo requirement tests
  byte-reproducibility.
- **[V]** `shasum -a 256` (macOS) and `sha256sum` (Linux) emit the same
  `hash  file` format `install.sh` parses with `awk` (`install.sh:24-26`). The
  exploratory release merges per-platform sums with `sort -u`, guarded by a `cmp` of
  `install.sh` copies (`ldist:.github/workflows/release.yml:25-34`).
- **[V]** One tag, one verify workflow and one publish job structurally let either
  platform block publication, provided each platform job is in `needs`
  (`release.yml:12-42`).

## 7. First Linux release vs real predecessor evidence

- **[V]** No releases or tags exist, so the first tagged release is the first for both
  platforms if they publish together.
- **[V]** The current `archive` step takes the first release other than the current tag
  and downloads only `climon2-*-macos-universal.tar.gz`, requiring exactly one match
  (`check.yml:157-172`). A predecessor without that asset fails the step; it does not
  fall back to an older release. For the first release it just echoes a message.
- **[V]** `CLIMON2_E2E_PREVIOUS_ARCHIVE` is validated to differ from the current version
  (`crates/climon2-e2e/tests/support/mod.rs:82-100`); the README says compatibility runs
  "from release two" and no evidence is fabricated for the first release
  (`crates/climon2-e2e/README.md:32,50`).
- **[X]** The exploratory workflow picks the newest release that has a same-platform
  asset, else prints "no synthetic compatibility fixture"
  (`ldist:.github/workflows/check.yml:214-240`; `ldist:docs/distribution.md:70-73`).
- **[D]** Cases (no policy chosen): (1) joint first release: neither platform has a
  predecessor; (2) later release with both assets: real same-platform predecessor;
  (3) a macOS-only release before Linux: a macOS archive is not a Linux predecessor and
  the current step would fail rather than skip; (4) prereleases (`--prerelease` for
  `-rc`) are listed and could be picked as "previous".

## 8. Exploratory branches (read-only, not proof)

- `linux` has an uncommitted `xtask/src/coverage.rs` change: on Linux
  `aligned_sections()` returns `-Cllvm-args=-runtime-counter-relocation`. No
  `quality.rs` change exists; no run evidence in the tree.
- `ldist` (`d1f03f9`) and `ltest` (`b77e6e4`) leave `quality.rs` and `coverage.rs`
  unchanged, yet `ldist:docs/distribution.md` says quality gates run on both OSes with
  "identical thresholds". Section 3 predicts other-OS cfg-inactive code with CC ≥ 2
  would score 0% without a filter or layout rule. Its Linux jobs apt-install
  `clang libclang-dev`, but `Cargo.lock` has no `bindgen`/`clang-sys` (unexplained).
- Shapes seen: mixed `#[cfg(target_os)]` inside shared bodies and out-of-line
  `#[path]` files, so both cases in section 3.2 occur.

## 9. Options (facts for #128-#130; nothing chosen)

- Native-source treatment: (a) run both tools on all files per OS and accept failures;
  (b) pre-filter the file/function list per OS from the cfg-active set, with a guard that
  compiled-but-uncovered code cannot be filtered out (crapper has no cfg input);
  (c) constrain layout so OS-specific bodies live in dedicated files; (d) merge LCOV from
  both OS runs for shared code (needs path normalisation; cannot prove native code).
  Whatever is chosen, "zero survivors" needs a companion check (section 3.3).
- Continuous coverage: per-OS flag (`-sectalign` vs `-runtime-counter-relocation`),
  with an executed exec-replaced and SIGKILL probe on Linux before relying on it.
- Portability: ubuntu-24.04-only release build, max-`GLIBC_*` assertion.
- Packaging: per-platform archive names and checksum tool, merged `SHA256SUMS`,
  deterministic GNU tar flags on Linux.
- Predecessor: "newest release with this platform's asset" vs "immediately previous
  release must contain it"; skip/fail behaviour is a #130 policy question.

## 10. Unresolved constraints and gaps

1. Linux continuous coverage for exec-replaced and SIGKILL processes is source-derived,
   not executed; upstream says Linux is "mostly complete but requires testing".
2. Unrun: cargo-llvm-cov 0.9.1 plus `-runtime-counter-relocation` across the CLI→client
   exec chain and a SIGKILLed daemon; multi-binary `%c%m` files; LTO internalization of
   the bias symbol against the repo's profiles.
3. Unrun: absence of `DA` records for cfg-inactive code in LCOV.
4. Hosted `ubuntu-24.04` suitability for PTY/vim/less/bash tests is unproven.
5. The symbol forcing `GLIBC_2.39` is unidentified.
6. Reproducibility requirement for Linux archives is unstated by the owner.
7. `attest-build-provenance@v2` vs `actions/attest@v4` equivalence unchecked.
8. Ubuntu 24.04 support-lifecycle dates were not fetched.
9. A Linux `.so` for the planned cdylib (`docs/runtime-artifacts.md`) is unspecified.
10. ADR 0003 (`:3,45`) assumes macOS-only bodies and a macOS-only full matrix; Linux
    gates or release would supersede part of it. This note proposes nothing.
