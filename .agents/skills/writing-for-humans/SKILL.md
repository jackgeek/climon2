---
name: writing-for-humans
description: Writing documents a human reads. Use when creating, editing, moving or removing `/README.md` or any document under `docs/` other than `docs/adr/` and `docs/agents/`.
---

Reference for every document a human reads: `/README.md` and the tree under
`docs/for-humans/`. Where a skill that owns a document type (`protocol-docs`)
differs from this one, that skill wins.

## Until the documents move

`docs/for-humans/` does not exist yet. Until a dedicated change creates it,
`docs/*.md` and `docs/protocol/` are the tree:

- Apply [Introducing terms](#introducing-terms) and [Diagrams](#diagrams) to
  every edit.
- Put new documents where their neighbours live today, linked from a document
  that `/README.md` already reaches.
- Leave existing files, folders and links where they are. Other branches are
  editing them, and a move now loses their work in merge conflicts.

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

For a term that `/GLOSSARY.md` or `docs/protocol/TERMS.md` defines, link its
first use to that definition. A short paraphrase beside the link is fine; the
definition's own words stay in the glossary, so the two cannot drift apart.
Change `/GLOSSARY.md` only through the `domain-modeling` skill.

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

1. What the folder covers, in one or two sentences.
2. Its audience. A subfolder inherits its parent's audiences and may narrow
   them; it never adds one its parent lacks. Material for a new audience moves
   up to the level where that audience is declared, because a reader does not
   search for their documents beneath someone else's.
3. The terms its children share.
4. A table with one row per file and subfolder: a link and a one-line purpose.
   Keep it complete: adding, renaming or removing a file updates the table in
   the same change.

### Links

Within the tree, a document links only to:

1. files in its own folder: its siblings and its parent;
2. files in any folder above it: its ancestors, aunts and uncles;
3. the `README.md` of a subfolder of its own folder or of a folder above it,
   which is the way into a branch the document is not in;
4. `docs/protocol/TERMS.md`.

So cousins reach each other through the README of the other's folder:
`protocol/daemon-client/request-open.md` links to
`protocol/monitor-client/README.md`, never to a page inside `monitor-client/`.
Siblings may link to each other in both directions. Everywhere else links run
one way; two documents that each need the other share material that belongs in
their parent.

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

`docs/crate-relationships.md` is the worked example.

## Before finishing

Read each changed document from top to bottom as its reader. Every term is
introduced above its first use, every link is one the family tree allows, and
`/README.md` still reaches every document you added or moved.
