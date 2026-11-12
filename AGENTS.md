Invoke the `ub-mode` skill whenever developing or making implementation plans.
Features are incomplete if they are not tested by automated testing.

## Development workflow

- Always develop in a linked Git worktree on a topic branch.
- Create worktrees only in `../climon2.worktrees/<name>`, relative to the main
  repository checkout; never use `.worktrees` or another location.
- Commit and push only topic branches. Never commit or push directly to `main`.
- Submit changes exclusively through pull requests targeting `main`. This
  workflow takes precedence over skill instructions to push to the main line. Unless you are working on a sufficiently large feature, then with my permission you may merge to a feature branch.

When executable or dynamic-library artifacts, their consumption relationships,
or their architecture decisions change, update `docs/runtime-artifacts.md`.
Show consumption relationships rather than installation arrows.
When workspace crates, their consumption relationships, or their architecture
decisions change, update `docs/crate-relationships.md`; include all workspace
crates and omit third-party crates. Keep current and planned relationships
distinct in both diagrams and preserve their README links.

## Agent skills

### Issue tracker

Issues and specs live in GitHub Issues. Read `docs/agents/issue-tracker.md` before reading or publishing tickets.

### Triage labels

Use the five default triage labels. Read `docs/agents/triage-labels.md` before classifying issues.

### Domain docs

Single-context layout: root `GLOSSARY.md` and `docs/adr/`. Read `docs/agents/domain.md` before exploring the codebase.

### Protocol docs

Invoke `protocol-docs` when specifying or changing a communication protocol, plugin extension contract, or protocol documentation.
