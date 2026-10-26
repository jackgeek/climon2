# Domain Docs

## Before exploring

This repository uses a single-context layout:

- Read root `GLOSSARY.md` for domain vocabulary.
- Read relevant ADRs in `docs/adr/` for decisions affecting the area of work.

If either is absent, proceed silently. The `/domain-modeling` skill creates domain docs lazily when terms or decisions are resolved.

## Use the glossary's vocabulary

Use glossary terms in issue titles, proposals, hypotheses, and test names. For an undefined concept, reconsider whether it belongs to the domain; note genuine gaps for `/domain-modeling`.

## Flag ADR conflicts

Explicitly surface proposals that contradict an ADR, identifying the decision and explaining why it is worth reopening.
