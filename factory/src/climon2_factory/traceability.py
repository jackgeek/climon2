"""Mechanical traceability: contract requirements ↔ scenarios ↔ acceptance tests."""

import re
from collections.abc import Mapping

SECTIONS = (
    "Purpose",
    "Behavior",
    "Scenarios",
    "Platforms and shells",
    "Out of scope",
    "Depends on",
    "Open questions",
)
_REQUIREMENT = re.compile(r"^\s*-\s+\*\*B(\d+)\.\*\*", re.MULTILINE)
_SCENARIO = re.compile(r"^\s*-\s+\*\*S(\d+)\*\*\s*(?:\(([^)]*)\))?", re.MULTILINE)
_CITATION = re.compile(r"\bB(\d+)\b")
_TEST_CITATION = re.compile(r"\bS(\d+)\b")


def contract_problems(contract: str) -> list[str]:
    problems = [
        f"contract has no '## {name}' section"
        for name in SECTIONS
        if not re.search(rf"^## {re.escape(name)}\s*$", contract, re.MULTILINE)
    ]
    requirements = [int(n) for n in _REQUIREMENT.findall(contract)]
    scenarios = [(int(n), cites) for n, cites in _SCENARIO.findall(contract)]
    if not requirements:
        problems.append("contract has no numbered requirements (- **B1.** …)")
    elif requirements != list(range(1, len(requirements) + 1)):
        problems.append("requirements are not numbered B1, B2, … without gaps")
    if not scenarios:
        problems.append("contract has no numbered scenarios (- **S1** (B1). …)")
    elif [n for n, _ in scenarios] != list(range(1, len(scenarios) + 1)):
        problems.append("scenarios are not numbered S1, S2, … without gaps")
    cited: set[int] = set()
    for number, cites in scenarios:
        refs = [int(b) for b in _CITATION.findall(cites)]
        if not refs:
            problems.append(f"scenario S{number} cites no requirement")
        problems += [
            f"scenario S{number} cites B{b}, which does not exist"
            for b in refs
            if b not in requirements
        ]
        cited.update(refs)
    problems += [
        f"requirement B{b} is not cited by any scenario"
        for b in requirements
        if b not in cited and scenarios
    ]
    return problems


def coverage_problems(contract: str, test_sources: Mapping[str, str]) -> list[str]:
    """Every contract scenario must be cited (as `S<n>`) by at least one test file."""
    cited = {int(n) for text in test_sources.values() for n in _TEST_CITATION.findall(text)}
    return [
        f"scenario S{n} is not cited by any test"
        for n in sorted({int(n) for n, _ in _SCENARIO.findall(contract)})
        if n not in cited
    ]
