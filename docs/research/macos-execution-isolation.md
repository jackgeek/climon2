# macOS execution isolation for the climon2 factory

## Decision status

Research only. No isolation design has been approved, installed, or tested.
The agreed runner arrangement remains GitHub-hosted Windows/Linux VMs and
direct local macOS validation. Moving macOS validation into a guest would
require a new decision.

Read-only local probes found an Intel Core i9 Mac, macOS 26.7.1,
`kern.hv_support=1`, 64 GiB RAM, and approximately 214 GiB available disk
space. `sandbox-exec` exists. No checked VM command was found on `PATH`;
this does not establish whether GUI virtualization applications are installed.

## Findings

| Option | Primary-source evidence | Assessment for this factory |
|---|---|---|
| App Sandbox | Apple documents filesystem/network confinement and signed command-line helpers inheriting a sandbox. [1][2] | A documented mechanism, but compatibility with Cargo, dynamically built helpers, PTY allocation, and the unchanged packaged binary has not been demonstrated. Not a verified drop-in solution. |
| `sandbox-exec` / Seatbelt profiles | Apple-published source contains a `sandbox-exec` test. [3] | Presence and internal usage do not establish a supported third-party policy interface or adequate confinement. Exact current deprecation wording was not verified. Treat custom profiles as experimental until support and behavior are established. |
| Dedicated standard user | Apple documents user accounts and file permissions; process permissions depend on identity. [4][5][6] | Practical partial containment while retaining native execution. Requires deliberate ownership/ACL separation for personal data, harness code, dependencies, and reports. Not a VM or an outbound-network sandbox. |
| Apple Virtualization.framework macOS guest | Apple's macOS platform configuration targets Apple silicon. [7] | Not an established Intel macOS guest path. Intel support elsewhere in the framework does not imply Intel macOS guest support. |
| Apple Hypervisor.framework | Apple documents Intel virtualization prerequisites and low-level VM facilities. [8] | Relevant hardware support exists, but this is not a complete guest installer or factory runner. |
| UTM, Tart, Lima macOS guests | Their documented macOS guest paths require Apple silicon/ARM. [9][10][11] | Not documented solutions for macOS guests on this Intel host. This finding does not rule out every custom QEMU configuration. |
| Parallels Desktop 26 | Official documentation lists Intel macOS guests, CLI automation, and guest execution facilities. [12][13][14][15] | A practical VM candidate, subject to licensing and compatibility validation. The reviewed Intel guest matrix lists macOS through Sequoia 15, not the host's reported 26.7.1. Host support must not be confused with guest support. |
| Docker Desktop / Linux VMs | Docker documents its Linux VM on macOS; Python describes PTY behavior as platform-dependent. [16][17] | Useful for Linux isolation, not evidence of native macOS PTY correctness. |

## Compilation must be contained too

Cargo executes build scripts, and procedural macros execute during compilation
with the compiler's resources. Sandboxing only the final executable leaves
build-time execution outside containment. Cargo's recommendation that build
scripts write only to `OUT_DIR` is not an enforcement mechanism. [18][19]

Agent tool permissions, a temporary directory, and a Git worktree do not
establish OS confinement for the native programs invoked through those tools.
This follows from the documented build-time execution and identity-based
filesystem/process permissions. [5][6][18][19]

## Host protection and test trust are separate requirements

A VM can separate guest execution from the host, but does not automatically
protect a same-user harness inside the guest. Candidate code must not have
write authority over trusted harness sources, imported dependencies, policy,
or authoritative reports. Same-user process interference is also relevant. [5][6]

Proposed requirements, not vendor guarantees or approved implementation:

- Separate candidate execution authority from trusted harness/report authority.
- Keep personal credentials, authentication agents, and unrelated files out of
  candidate execution environments.
- Record candidate commit, harness revision, actual OS/architecture, package and
  executable hashes, and the trusted collector's observations.
- Do not accept candidate-generated "passed" messages or hashes as proof.
- Export only bounded, expected artifacts through a narrow channel; treat paths,
  symlinks, archives, logs, and terminal escape sequences as untrusted input.
- Verify timeout and descendant cleanup independently of pytest's completion.

For a VM, disable unnecessary shared folders, clipboard integration, device
sharing, and credential forwarding; define networking and artifact export
explicitly. Parallels documents host/guest integrations, so an isolation claim
must depend on verified configuration rather than the mere presence of a VM.
Snapshot rollback cannot undo earlier information disclosure or host writes.
The latter is an inference from the scopes of snapshots and host integration. [13][20]

For a dedicated user, inspect permissions rather than assuming another account
cannot read shared/world-readable data. Apple's application firewall concerns
incoming connections and is not evidence of outbound-exfiltration protection.
Manual approval accepts execution risk; it does not create confinement. [4][5][21]

## Operational limits and unknowns

Apple documents resource limits, but they must not be mistaken for a complete
aggregate job resource budget. Descendants can create new sessions/process
groups, so terminating an original process group is not a universal cleanup
guarantee. [22][23]

Parallels CLI automation requires an appropriate edition; documented guest
command execution requires Parallels Tools. Setup, licensing, administrator
requirements, unattended guest startup, and PTY behavior on this exact host
remain untested. [14][15]

Before choosing an option, establish:

1. Whether direct native execution can provide enforceable restrictions for
   both the complete build chain and terminal tests without altering the
   packaged executable.
2. Whether a supported Intel macOS guest is available, and whether validating a
   different macOS version is acceptable.
3. How trusted harness execution is privilege-separated from candidate code,
   including inside a guest.
4. How networking, output size, disk use, timeouts, escaped descendants, and
   cleanup are controlled.
5. Whether the installed Apple documentation establishes a supported role for
   custom Seatbelt profiles; this research did not establish one.

## Recommendation

Do not claim that a worktree or scoped agent tools make unattended native
execution safe.

A dedicated non-admin account with protected harness/report ownership is an
incremental direct-native option, but its limitations must be accepted
explicitly. For stronger host separation, evaluate a supported Intel macOS VM
product such as Parallels, first resolving guest-version support and the change
to the agreed direct-local runner plan.

Neither route is approved or proven for this factory. Do not introduce an
experimental sandbox profile as a security guarantee. Compatibility and denial
tests can establish evidence for a specific configuration, not absolute safety.

## Primary sources

1. [Apple: Protecting user data with App Sandbox](https://developer.apple.com/documentation/security/protecting-user-data-with-app-sandbox)
2. [Apple: Embedding a command-line tool in a sandboxed app](https://developer.apple.com/documentation/xcode/embedding-a-helper-tool-in-a-sandboxed-app)
3. [Apple source: sandbox-exec test, fixed revision](https://github.com/apple-oss-distributions/adv_cmds/blob/6bed8737a34dbb54782a18f47dccf933a9967a12/genwrap/tests/genwrap_sandbox_test.sh#L42-L55)
4. [Apple: Users & Groups settings](https://support.apple.com/guide/mac-help/change-users-groups-settings-mtusr001/mac)
5. [Apple: File, folder, and disk permissions](https://support.apple.com/guide/mac-help/change-permissions-for-files-folders-or-disks-mchlp1203/mac)
6. [Apple XNU: kill permission rules, fixed revision](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/bsd/man/man2/kill.2#L67-L74)
7. [Apple: VZMacPlatformConfiguration](https://developer.apple.com/documentation/virtualization/vzmacplatformconfiguration)
8. [Apple: Hypervisor framework](https://developer.apple.com/documentation/hypervisor)
9. [UTM: macOS guest support](https://docs.getutm.app/guest-support/macos/)
10. [Tart: official overview](https://tart.run/)
11. [Lima: macOS guests](https://lima-vm.io/docs/usage/guests/macos/)
12. [Parallels: system requirements and guest matrices](https://kb.parallels.com/en/124223)
13. [Parallels 26: running macOS virtual machines](https://docs.parallels.com/landing/pdfm-ug/v26-en-us/parallels-desktop-for-mac-26-users-guide/advanced-topics/using-other-operating-systems-on-your-mac/running-macos-virtual-machines)
14. [Parallels: command-line interface](https://docs.parallels.com/landing/parallels-desktop-developers-guide/command-line-interface-utility)
15. [Parallels: executing a guest command](https://docs.parallels.com/landing/parallels-desktop-developers-guide/command-line-interface-utility/manage-virtual-machines-from-cli/general-virtual-machine-management/execute-a-command-in-a-virtual-machine)
16. [Docker: Mac permissions and Linux VM](https://docs.docker.com/desktop/setup/install/mac-permission-requirements/)
17. [Python: platform-dependent PTY support](https://docs.python.org/3/library/pty.html)
18. [Cargo: build scripts](https://doc.rust-lang.org/cargo/reference/build-scripts.html)
19. [Rust Reference: procedural macros](https://doc.rust-lang.org/reference/procedural-macros.html)
20. [Parallels 26: snapshots](https://docs.parallels.com/landing/pdfm-ug/v26-en-us/parallels-desktop-for-mac-26-users-guide/advanced-topics/working-with-virtual-machines/saving-snapshots-of-a-virtual-machine)
21. [Apple: application firewall](https://support.apple.com/guide/mac-help/block-connections-to-your-mac-with-a-firewall-mh34041/mac)
22. [Apple XNU: resource limits, fixed revision](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/bsd/man/man2/getrlimit.2#L56-L132)
23. [Apple XNU: setsid, fixed revision](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/bsd/man/man2/setsid.2#L48-L61)
