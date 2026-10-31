# Design review prompt

Review the supplied design against its intended outcomes, requirements, constraints, and applicable standards. The design may be a proposal, architecture, interface, workflow, system, or implementation. Adapt the review to its domain and maturity.

## Establish scope

Identify the design artifacts, intended users, success criteria, explicit non-goals, and authoritative requirements or decisions. Follow relevant references. State assumptions where context is missing; ask for clarification only when it prevents a meaningful review.

Distinguish the proposed design from existing behavior and future work. Use implementation evidence when available, without treating an unimplemented proposal as a defect.

## Review

Evaluate the design along two axes:

- **Requirements:** Does it achieve the intended outcomes? Trace each material requirement to a design mechanism or identify a gap. Examine normal use, boundary cases, failure and recovery paths, and interactions with surrounding systems.
- **Design quality:** Does it follow applicable standards and make defensible tradeoffs? Examine clarity, consistency, ownership, dependencies, simplicity, maintainability, and how its claims can be validated. Consider reliability, security, accessibility, performance, cost, compatibility, and operational concerns where relevant.

For each material risk, describe a concrete scenario, the mechanism that produces the problem, and the consequence. Challenge load-bearing assumptions and compare plausible alternatives against the stated constraints.

Validate claims through available evidence: document references, worked examples, calculations, experiments, or tests. Distinguish demonstrated defects, reasoned risks, and unanswered questions. Report actual checks and their results; label proposed checks as unperformed.

Recommend the smallest correction that addresses the cause while preserving intended behavior. Explain any necessary requirement change or tradeoff. Keep optional improvements separate from blockers; avoid findings based solely on personal preference.

## Report

Lead with a verdict: **ready**, **ready with conditions**, **needs revision**, or **insufficient evidence**. Briefly explain the decisive reason.

Present **Requirements** and **Design quality** findings separately, ordered by impact. For each finding include:

- **Priority:** P1 blocks the intended outcome or creates unacceptable risk; P2 is a material issue to address; P3 is an optional improvement.
- **Location:** a precise artifact, section, diagram element, interface, or file/line reference.
- **Problem and impact:** the concrete scenario and affected requirement or design property.
- **Evidence and confidence:** what supports the claim and what remains uncertain.
- **Correction and validation:** the smallest justified change and how to establish that it works.

Finish with unresolved decisions, review limits, and a count of findings and highest priority for each axis. If an axis has no substantiated findings, say so. A clean review with limited evidence is not proof that the design is correct.

Review only. Leave the design and its implementation unchanged unless explicitly asked to revise them.
