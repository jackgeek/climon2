# Handoff: PR #181 / #171 Prove the client is past restoration before typing in the older e2e scenarios

Worktree `t171`, branch `impl/171-continuity-typing-proof`, head `2e5d793` (pushed; merges `feature/monitor-impl` `6930b0e` cleanly). Not on the POC path.

- **Independent review 2026-11-14: VERDICT MERGEABLE**, no hard findings. It is test-only (four climon2-e2e files), so there is no CRAP or mutation exposure.
  - Minor findings: 6.1.2 now quits via a file, not a typed key; the generic fixture's oracle proves the Enter arrived, not the secret; the acknowledger cleanup is proven only by macOS CI.
  - The previous-release scenario is covered by CI (check.yml sets `CLIMON2_E2E_PREVIOUS_ARCHIVE`).
- **Gate:** none yet; it was queued and cancelled by the stop.
- **Next:** ticket gate, CI on all runners (watch macOS), squash-merge, close #171.
