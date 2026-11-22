---
name: writing-for-humans
description: Writing documents a human reads. Use when creating, editing, moving or removing `/README.md` or any document under `docs/for-humans/`.
---

Reference for every document a human reads: `/README.md` and the tree under
`docs/for-humans/`. Where a skill that owns a document type (`protocol-docs`)
differs from this one, that skill wins.

## The reader

Write for a competent developer who has never seen this repository. They know
general terminal and Rust vocabulary and none of climon2's terms. Each folder
declares which **audience** of such developers it serves:

- **Users** run climon2.
- **Integrators** write clients or plugins against the protocol.
- **Contributors** change climon2's code.

When a document fits none of these, propose a new audience to the user as an
edit to this list.

## Introducing terms

Introduce every term before the first sentence that uses it. Build compound
terms from their parts: a document about flangewanglers first says what a
flange is, then what it means to wangle one, and only then says
"flangewangler".

A term counts as introduced when an ancestor of the document (see
[The document tree](#the-document-tree)) introduced it. When several siblings
need the same term, introduce it once in their parent. When the parent is the
wrong place for it, add a folder between them whose `README.md` introduces it.

A **reference** is a document of definitions that a folder `README.md`
declares, with one line linking it:

```markdown
**Reference:** [technical terms](TERMS.md)
```

Its terms count as introduced throughout that folder and below, provided each
document links a term's first use to its definition. A short paraphrase beside
the link is fine; the definition's own words stay in the reference, so the two
cannot drift apart. A declared reference lives in the folder it serves, or
outside the tree. `docs/for-humans/README.md` declares `/GLOSSARY.md`, the
domain glossary, for the whole tree; change that one only through the
`domain-modeling` skill. Declare a new reference when a branch has enough
vocabulary of its own that its parent would be the wrong place to introduce
it.

## The document tree

Documents form a **family tree** that matches the folder tree. General
material sits at the top and each branch narrows to one detail.

- `/README.md` is the root. It needs no table and may link to any document in
  the tree.
- Every folder has a `README.md`, the **parent** of the files and subfolders
  beside it. Files in one folder are **siblings**. The READMEs of the folders
  above are a document's **ancestors**.
- An **orphan** is a document that no chain of links from `/README.md`
  reaches. Leave no orphans.

### Folder READMEs

A folder `README.md` holds, in order:

1. What the folder covers, in one or two sentences, and any `**Builds on:**`
   declaration (see [Links](#links)).
2. Its audience, as one line: `**Audience:** Integrators and Contributors.`
   A subfolder inherits its parent's audiences and may narrow them; it never
   adds one its parent lacks. Material for a new audience moves
   up to the level where that audience is declared, because a reader does not
   search for their documents beneath someone else's.
3. The terms its children share, and any `**Reference:**` declaration (see
   [Introducing terms](#introducing-terms)).
4. A table with one row per file and subfolder: a link and a one-line purpose.
   Keep it complete: adding, renaming or removing a file updates the table in
   the same change.

### Links

The top-level folders of `docs/for-humans/` are **subjects**: `protocol/`,
`behaviour/`, `architecture/`, `windows/`. These rules apply inside one subject; a link
from one subject to another is a citation between specifications and is
unrestricted. Within a subject, and from the files at the top of the tree, a
document links only to:

1. files in its own folder: its siblings and its parent;
2. files in any folder above it: its ancestors, aunts and uncles;
3. the `README.md` of a subfolder of its own folder or of a folder above it,
   which is the way into a branch the document is not in;
4. anything below its own folder: links straight down follow the tree, as
   links straight up do;
5. anything in a sibling folder that its folder, or a folder above it, declares
   it builds on;
6. a page that links to it under such a declaration: a **reply** to that
   link.

A folder declares that it builds on a sibling folder in its `README.md`, with
one line linking the sibling's README:

```markdown
**Builds on:** [daemon-client](../daemon-client/README.md)
```

Use it when one folder's pages are defined in terms of the other's, as the
monitor-client calls relay daemon-client messages. The declaration runs one
way, and two folders never declare each other. The declared folder gains only
replies: `daemon-client/request-focus.md` may link to
`monitor-client/request-focus.md` because that page links to it first. A
reply cannot open a path that the declaration has not already opened.

Links run straight up or straight down. Only a sideways link, to a cousin in
another branch, goes through that branch's README:
`protocol/daemon-client/request-open.md` links to
`protocol/monitor-client/README.md`, never to a page inside `monitor-client/`.

Links that leave the tree are unrestricted: `/GLOSSARY.md`, ADRs, crate
READMEs, source files, issues and external pages.

## Diagrams

Each Mermaid diagram answers one question:

1. State its scope in a sentence above it: what it shows, what it leaves out,
   and what its arrows mean.
2. When it would mix groups a reader thinks about separately (shipped and
   development crates, runtime and installation), draw one diagram per group.
   Each relationship appears in exactly one diagram. A node repeated for
   context is marked as described elsewhere.
3. Below it, keep a table with one row per node giving its purpose, so the
   diagram itself carries only names.

`docs/for-humans/architecture/crate-relationships.md` is the worked example.

## Before finishing

Read each changed document from top to bottom as its reader. Every term is
introduced above its first use, every link is one the family tree allows, and
`/README.md` still reaches every document you added or moved.
`python3 docs/for-humans/check.py` checks the tree, the tables, the audiences,
the declarations and the links; CI runs it on every change to a Markdown file.
