# Content-addressed artifacts with recorded input hashes

Every factory output (contract, findings, tests, red evidence, candidate evidence) is identified by its content hash. Its manifest records the hashes of the inputs it was derived from, including role prompts and skills. Approvals bind to one artifact hash. Validity is always recomputed from current hashes and never cached, so any upstream edit reliably makes downstream artifacts and approvals stale. Superseded artifacts are kept as history.

## Considered Options

- **Timestamps or step order:** rejected because direct user edits and reruns silently invalidate them.
- **Cached "valid" flags:** rejected because a missed invalidation path would let unapproved tests or contracts advance a run.
