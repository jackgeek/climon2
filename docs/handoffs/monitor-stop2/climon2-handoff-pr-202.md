# Handoff: PR #202 Monitor: EIO on host input is the closed host terminal (load flake)

Worktree `fix-hangup-flake`, branch `fix/monitor-hangup-flake`, head `8f6db3c` (pushed), base `feature/monitor-impl` `6930b0e`. Draft.

- **Root cause** of the gate flake `a_hung_up_host_input_releases_the_host_before_its_diagnostic`: Linux returns EIO from the PTY slave while it hangs it up, and the monitor treated EIO as an error rather than the closed host terminal. Fixed test-first in 5b71943, with a stress run of 24 suites and 0 failures.
- **CI at 5b71943 failed everywhere** in the new EIO test. With `set -m`, bash 3.2/5.2 print `[1]+ Exit 125 …` on the outer shell's stderr after the diagnostic (local bash 5.3 does not).
  - 8f6db3c sends the outer shell's stderr to /dev/null and the monitor's stderr via fd 3; no assertion changed.
  - **CI run 43616633890 on `8f6db3c` was queued at the stop and is unverified.**
  - Cosmetic: 8f6db3c's message shows doubled backslashes.
- **Next:**
  1. If CI is green, run the ticket gate, then CI on the merged head, then squash-merge.
  2. Other branches inherit the fix through their next `feature/monitor-impl` merge.
