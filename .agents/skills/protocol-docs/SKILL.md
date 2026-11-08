---
name: protocol-docs
description: Maintain protocol documentation when specifying or changing wire formats, message behavior, compatibility rules, plugin extension contracts, or protocol documentation.
---

# Protocol docs

Maintain a Markdown reference that future plugin authors can understand without
reading climon2's implementation. Every protocol modification must update this
reference in the same change.

## Workflow

1. Establish the contract. Read the relevant agreed specifications, ADRs,
   implementation, contract tests, root `GLOSSARY.md`, and protocol `TERMS.md`.
   Read `docs/agents/issue-tracker.md` before fetching specifications from issues.
   Distinguish agreed but unimplemented behavior from shipped behavior. Mark
   unresolved details explicitly and link their decision tickets.

2. Maintain `docs/protocol/README.md` as the entry point. Explain which programs
   communicate, the basic framing and message structure for each interface,
   startup and capability negotiation, and how to read the message reference.
   Link the domain glossary, technical terms, and a separate Markdown page for
   every message type. Group the message index by interface and show each
   message's direction and purpose. Keep message-specific details on their
   own pages.

3. Maintain message pages under `docs/protocol/<interface>/`. Use distinct
   filenames for requests, responses, and events where names overlap. When one
   message type carries several operations, give each operation its own page.
   Each page must explain:
   - Purpose, sender, receiver, and when the message is allowed.
   - Required and optional fields, types, values, defaults, units, and limits.
   - Text and binary encodings, payload framing, and chunking where applicable.
   - Ordering, request IDs, control periods, restoration, and readiness rules
     relevant to this message.
   - What acceptance or acknowledgement proves, and what it does not prove.
   - Errors, rejection behavior, and effects on the connection or session.
   - Capability requirements, compatible extensions, and unknown-field behavior.
   - A valid example and any binary payload explained separately.
   - Links to related messages and the specification that owns the contract.

4. Maintain domain concepts in root `GLOSSARY.md`, the single domain glossary
   for this repository. Invoke `domain-modeling` when changing those concepts.
   Define non-intuitive technical vocabulary in `docs/protocol/TERMS.md`;
   keep wire schemas and implementation details in the message reference.
   Link both references from the overview and every message page. At the first
   prose occurrence of each domain term, non-intuitive technical term, or
   abbreviation in each document, link the term itself to its owning definition
   (using an anchor where available). Keep definitions and abbreviation
   expansions solely in those references, not inline in the overview or
   message pages. A later explanation or a general reference link does not
   satisfy first-use linking.

5. Synchronize the change. Update every affected message page, the overview,
   related examples, compatibility guidance, and affected terminology together.
   Document plugin extension negotiation, relay, interception, and validation
   guarantees where relevant. Preserve the distinction between protocol support
   for future plugins and an implemented plugin system.

6. Verify completeness. Every documented message and operation is reachable
   from the overview; relative links resolve; examples match the stated
   contract; and fields, limits, ordering, and failure behavior agree with the
   owning specification and implementation status. Read each changed document
   from top to bottom: verify first-use term links appear before subsequent
   mentions and that prose contains no duplicated definitions or abbreviation
   expansions. Use existing documentation checks and contract tests where
   available. Report unresolved discrepancies rather than describing planned
   behavior as implemented.

## Language

Use simple, informative language, short sentences, and concrete examples.
Link abbreviations to their technical definitions on first use. Describe
observable behavior precisely without requiring knowledge of internal functions
or crates. Define shared structure once in the overview and link to it from
message pages.
