"""Check the human-read document tree against the writing-for-humans skill.

Fails on an orphan, a folder without a README.md, a README without an
audience or with an audience its parent lacks, a folder table missing a file,
a `Builds on` declaration that is not one-way between siblings, and a link
the family-tree rule forbids. The rule applies inside each top-level subject
folder; links between subjects are unrestricted. Run with `python3 docs/for-humans/check.py`.
"""

import os
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
ROOT_README = ROOT / "README.md"
TERMS = HERE / "protocol" / "TERMS.md"
AUDIENCES = {"Users", "Integrators", "Contributors"}

LINK = re.compile(r"\]\(([^)\s]+)\)")
AUDIENCE = re.compile(r"^\*\*Audience:\*\*(.*)$", re.MULTILINE)
BUILDS_ON = re.compile(r"^\*\*Builds on:\*\*(.*)$", re.MULTILINE)
FENCE = re.compile(r"^```.*?^```", re.MULTILINE | re.DOTALL)


def links(page):
    """Yield (line, resolved path) for each relative link outside code fences."""
    text = FENCE.sub("", page.read_text())
    for line in text.splitlines():
        for target in LINK.findall(line):
            if re.match(r"[a-z][a-z0-9+.-]*:", target) or target.startswith("#"):
                continue
            path = target.split("#", 1)[0]
            if not path:
                continue
            resolved = Path(os.path.normpath(page.parent / path))
            if resolved.is_dir():
                resolved = resolved / "README.md"
            yield line, resolved


def inside(path, folder):
    return folder == path or folder in path.parents


def folders():
    return sorted(p for p in [HERE, *HERE.rglob("*")] if p.is_dir() and "__pycache__" not in p.parts)


def subject(path):
    """The top-level folder under docs/for-humans/ holding path, or None."""
    parts = path.relative_to(HERE).parts
    return parts[0] if len(parts) > 1 else None


def audiences(readme):
    match = AUDIENCE.search(readme.read_text()) if readme.exists() else None
    if not match:
        return None
    words = re.split(r",|\band\b", match.group(1))
    return {w.strip(" .") for w in words if w.strip(" .")}


def builds_on(folder):
    readme = folder / "README.md"
    if not readme.exists():
        return set()
    match = BUILDS_ON.search(readme.read_text())
    if not match:
        return set()
    return {
        Path(os.path.normpath(readme.parent / t.split("#", 1)[0])).parent
        for t in LINK.findall(match.group(1))
    }


def check():
    errors = []
    dirs = folders()
    declared = {d: builds_on(d) for d in dirs}

    for folder in dirs:
        readme = folder / "README.md"
        rel = folder.relative_to(ROOT)
        if not readme.exists():
            errors.append(f"{rel}: folder has no README.md")
            continue
        found = audiences(readme)
        if found is None:
            errors.append(f"{rel}/README.md: no **Audience:** line")
        elif not found <= AUDIENCES:
            errors.append(f"{rel}/README.md: unknown audience {sorted(found - AUDIENCES)}")
        elif folder != HERE:
            parent = audiences(folder.parent / "README.md") or set()
            if not found <= parent:
                errors.append(f"{rel}/README.md: audience {sorted(found - parent)} not in parent")
        tabled = {target for line, target in links(readme) if line.lstrip().startswith("|")}
        for child in sorted(folder.iterdir()):
            if child.name in {"README.md", "__pycache__"}:
                continue
            want = child / "README.md" if child.is_dir() else child
            if want not in tabled:
                errors.append(f"{rel}/README.md: table lacks {child.name}")
        for other in declared[folder]:
            if other.parent != folder.parent or other == folder or not other.is_dir():
                errors.append(f"{rel}/README.md: Builds on {other.relative_to(ROOT)} is not a sibling folder")
            elif folder in declared.get(other, set()):
                errors.append(f"{rel}/README.md: Builds on {other.relative_to(ROOT)}, which builds on it")

    pages = sorted(HERE.rglob("*.md"))
    for page in pages:
        src = page.parent
        granted = [g for d, gs in declared.items() if inside(src, d) for g in gs]
        for _, target in links(page):
            if not inside(target, HERE):
                continue
            tdir = target.parent
            allowed = (
                tdir == src
                or tdir in src.parents
                or target == TERMS
                or (target.name == "README.md" and (tdir.parent == src or tdir.parent in src.parents))
                or (page.name == "README.md" and inside(target, src))
                or any(inside(target, g) for g in granted)
                or (subject(page) and subject(target) and subject(page) != subject(target))
            )
            if not allowed:
                errors.append(f"{page.relative_to(ROOT)}: link to {target.relative_to(ROOT)} breaks the family rule")
            elif not target.exists():
                errors.append(f"{page.relative_to(ROOT)}: link to missing {target.relative_to(ROOT)}")

    reached, queue = set(), [ROOT_README]
    while queue:
        page = queue.pop()
        if page in reached:
            continue
        reached.add(page)
        queue.extend(t for _, t in links(page) if t.suffix == ".md" and inside(t, HERE) and t.exists())
    errors += [f"{p.relative_to(ROOT)}: orphan, not reached from /README.md" for p in pages if p not in reached]
    return errors


if __name__ == "__main__":
    problems = check()
    print("\n".join(problems))
    sys.exit(1 if problems else 0)
