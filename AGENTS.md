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
Its diagram shows only the runtime artifacts that execute when a user types `c2`
followed by a command, and the climon2 executables and libraries those calls
ultimately invoke, with their consumption relationships. Never add installation
(the installer or install arrows) or development and test-only executables
(xtask, Cargo tooling, fixtures, test harnesses, stand-in or second-release
libraries) to it.
When workspace crates, their consumption relationships, or their architecture
decisions change, update `docs/crate-relationships.md`. Include all workspace
crates and omit third-party crates. It holds two diagrams:

- **Shipped crates:** only crates that ship, and only the normal dependencies
  between them. Never show development or test-only crates or dev-dependencies
  here.
- **Development and test-only crates:** every edge that starts or ends at a
  development or test-only crate, including shipped crates' dev-dependencies on
  them. Show a shipped crate only when such an edge touches it, inside the
  shipped subgraph, and never draw an edge between two shipped crates.

Every workspace crate belongs to exactly one group; a new crate that ships goes
in the first diagram, and one used only for development or tests goes in the
second. A planned crate goes in the diagram of the group it will join. Derive
current edges from `cargo metadata --no-deps --format-version 1`, not memory,
and make sure each workspace-crate edge appears in exactly one diagram.

Keep current and planned relationships distinct in the crate and artifact
diagrams and preserve their README links.

## Agent skills

### Issue tracker

Issues and specs live in GitHub Issues. Read `docs/agents/issue-tracker.md` before reading or publishing tickets.

### Triage labels

Use the five default triage labels. Read `docs/agents/triage-labels.md` before classifying issues.

### Domain docs

Single-context layout: root `GLOSSARY.md` and `docs/adr/`. Read `docs/agents/domain.md` before exploring the codebase.

### Protocol docs

Invoke `protocol-docs` when specifying or changing a communication protocol, plugin extension contract, or protocol documentation.
