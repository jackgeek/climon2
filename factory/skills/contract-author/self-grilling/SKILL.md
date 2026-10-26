---
name: self-grilling
description: Interrogate a brief relentlessly before writing a contract, when no human is available to answer. Use when drafting or revising a feature contract.
---

# Self-grilling

There is no one to ask during your step. Grill the brief yourself instead, one question at a time:

1. Walk the brief branch by branch: inputs, outputs, errors, exit codes, platforms, concurrency, limits, and what happens at every boundary.
2. For each question, look for the answer in the brief, `CONTEXT.md`, the ADRs in `docs/adr/` and earlier features' contracts. Quote where you found it.
3. If the sources settle it, write it into the contract as a behaviour requirement.
4. If they don't, choose the most conservative answer that is consistent with the sources, write it in, and list it under `## Open questions` with your recommendation, so the reviewer and the user can overturn it.

Never invent scope the brief doesn't ask for. If something is ambiguous, make it explicit; don't leave it implicit.
