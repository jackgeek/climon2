"""Mechanical contract and test traceability checks."""

from climon2_factory.traceability import contract_problems, coverage_problems

VALID = """# Contract: f1

## Purpose
Run a command in a session.

## Behavior
- **B1.** `climon2 exec cmd` runs cmd.
- **B2.** `climon2 list` shows it.

## Scenarios
- **S1** (B1). Given …, when …, then ….
- **S2** (B1, B2). Given …, when …, then ….

## Platforms and shells
All.

## Out of scope
Nothing.

## Depends on
None.

## Open questions
None.
"""


def test_a_well_formed_contract_has_no_problems() -> None:
    assert contract_problems(VALID) == []


def test_missing_sections_are_named() -> None:
    text = VALID.replace("## Out of scope\nNothing.\n\n", "").replace("## Purpose\n", "")

    assert contract_problems(text) == [
        "contract has no '## Purpose' section",
        "contract has no '## Out of scope' section",
    ]


def test_a_contract_needs_requirements_and_scenarios() -> None:
    text = (
        VALID.split("## Behavior")[0]
        + "## Behavior\n\n## Scenarios\n\n## Platforms"
        + (VALID.split("## Platforms")[1])
    )

    assert contract_problems(text) == [
        "contract has no numbered requirements (- **B1.** …)",
        "contract has no numbered scenarios (- **S1** (B1). …)",
    ]


def test_numbering_gaps_are_reported() -> None:
    text = VALID.replace("**B2.**", "**B3.**").replace("(B1, B2)", "(B1, B3)")

    assert contract_problems(text) == ["requirements are not numbered B1, B2, … without gaps"]


def test_scenarios_must_cite_existing_requirements() -> None:
    text = VALID.replace("**S1** (B1).", "**S1**.").replace("(B1, B2)", "(B1, B2, B9)")

    assert contract_problems(text) == [
        "scenario S1 cites no requirement",
        "scenario S2 cites B9, which does not exist",
    ]


def test_every_requirement_must_be_cited() -> None:
    text = VALID.replace("(B1, B2)", "(B1)")

    assert contract_problems(text) == ["requirement B2 is not cited by any scenario"]


def test_every_scenario_must_be_cited_by_a_test() -> None:
    sources = {"tests/test_exec.py": 'def test_runs():\n    """S1 (B1): runs."""\n'}

    assert coverage_problems(VALID, sources) == ["scenario S2 is not cited by any test"]
    sources["tests/test_list.py"] = 'def test_lists():\n    """S2: lists."""\n'
    assert coverage_problems(VALID, sources) == []


def test_s1_is_not_cited_by_s10() -> None:
    sources = {"tests/test_x.py": '"""S10, S2"""\n'}

    assert coverage_problems(VALID, sources) == ["scenario S1 is not cited by any test"]
