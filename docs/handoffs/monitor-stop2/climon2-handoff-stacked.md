# Handoff: stacked POC branches without PRs (#83, #89, #93, #94, #106, #108, #109)

All are pushed and clean, with no gate run. Each opens a draft PR to `feature/monitor-impl` once its bases have landed, then merges the new base and gets a ticket gate. Hand-resolved code merges mean every one of these needs its own gate.

| Ticket | Worktree / branch | Head | Built on | State |
|---|---|---|---|---|
| #83 read-only vt views | t83 / impl/83-read-only-vt-views | `d31edfa` | #76 | Review fixes and owner decisions done (READY stacked). Waits for #159. |
| #89 monitors acquire and inherit control | t89 / impl/89-monitors-acquire-and-inherit-control | `4c10181` | #88 at 0536509 | Review items 1-4 and headless tests done. Must merge #88's final head; drop the old `publish()` calls in session.rs. Follow-up #184. |
| #93 passive connections through the library | t93 / impl/93-passive-monitor-connections-through-the-library | `f7035b1` | #85 (now landed) | All review items done. Merge `feature/monitor-impl`; expect hand resolution in client library internals, then gate. |
| #94 control and display through the library | t94 / impl/94-library-control-and-display | `7ed6002` | #89, #93 | Review fixes and owner decisions done: Command final relayed, clean stop after a known result, Display split to chunk_bytes, stall retry passive, real-daemon Leave race. |
| #106 discovery and basic selector | t106 / impl/106-discovery-and-basic-selector | `0f23fac` | #84, #93, #105, #77 | Review fixes and owner decisions done: stale never-connected rows dropped, Escape gap closed by #109, stall_ms belongs to #120. |
| #108 select and paint | t108 / impl/108-select-and-paint | `9a0935e` | #106, #94, #83, #76 | Review fixes and owner decisions done: top row fixed slots separated by `\|`, too small under 4×5, Controlling kept while covered, test-term back to a 2-row minimum, whole-frame test waits. The memory double count is an approved gap on #120. |
| #109 input router | t109 / impl/109-input-router | `4443ec3` | #108 | Owner decisions done: Escape has no timer (passes through; modals wait for the next byte), accepted input drains before exit within operation_ms, paint-then-route after a modal closes, host-reply rule confirmed. |

**Landing order:** #159 (#76) → #83; #182 (#88) → #89; #175 (#84); then #93 → #94 → #106 → #108 → #109. Wherever the stack allows, land a base before its dependants. Merge with the stacked branch's latest head.

**Not yet done:**
- the 4-way stress run of the #108 and #109 packaged tests (needs the gate lock);
- the #108 whole-frame waits are not proven under load;
- #110-#122 and the attention work (#91, #92, #95, #96, #107) are outside the POC.

**Key reviews:** codereview-94.md and codereview-106.md in the session files. #108's review findings are summarised on #108.
