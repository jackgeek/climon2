# Handoff: PR #175 / #84 One validated loader for client libraries

Worktree `t84`, branch `impl/84-validated-client-library-loader`, head `e372d5b` (pushed; merges `feature/monitor-impl` `6930b0e`).

- **CI run 43611753497 on `e372d5b`: green on every runner.** The macOS-only mutation job reports `macos.rs` 8 killed, 0 survived, max CRAP 4.0.
- **Root cause of the old macOS leaks failure:** test-term only claimed detached processes through 25 ms lineage sampling. Fixed by giving macOS the inherited ownership lease (7986805), with regression test 8eec96e; the fixture detach race was fixed in 06661ee.
- **Owner decisions:**
  - Option 1, a contract test killing the image-0 mutant (854b17f).
  - Option A, the loader passes a sentinel output: an INTERNAL_ERROR bootstrap must clear it, otherwise the load is a malformed description (e372d5b). Documented in core-v1 and library-embedding.
- **Re-gate required:** merge 5463075 hand-resolved non-Markdown conflicts with #85.
- **Gate:** none yet. Ignore the empty `target/gate-logs/climon2-gate-t84-tk1.log`, left by a killed duplicate.
- **Next:** ticket gate, CI on the merged head, squash-merge, close #84.
- Commits 8eec96e and 06661ee carry an old Claude trailer (pushed before the rule; not rewritten).
