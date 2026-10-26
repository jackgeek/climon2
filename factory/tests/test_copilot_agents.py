"""How a step is handed to a Copilot session. Deterministic: the session itself is faked."""

from pathlib import Path

import pytest

from climon2_factory.copilot_agents import MODELS, CopilotAgents, Session
from climon2_factory.ports import (
    AgentRequest,
    ContractSubmitted,
    Finding,
    ReviewSubmitted,
    Role,
    Severity,
)
from climon2_factory.workflow import StepFailed


def no_locks(workspace: Path, project: str) -> str:
    raise AssertionError("not expected")


class Recorder:
    def __init__(self, act: str | None = None, **arguments: object) -> None:
        self.sessions: list[Session] = []
        self._act, self._arguments = act, arguments

    async def __call__(self, session: Session) -> None:
        self.sessions.append(session)
        if self._act:
            session.tools[self._act](**self._arguments)


def test_contract_author_gets_its_model_prompt_skills_and_tools(tmp_path: Path) -> None:
    converse = Recorder("factory_submit_contract")
    agents = CopilotAgents(no_locks, converse=converse, time_limit_s=42)

    submission = agents.run(AgentRequest(Role.CONTRACT_AUTHOR, "f1", "draft-contract", tmp_path))

    assert submission == ContractSubmitted()
    [session] = converse.sessions
    assert session.model == MODELS[Role.CONTRACT_AUTHOR] == "gpt-6-astra"
    assert "Role: Contract Author" in session.instructions
    assert "you have no shell" in session.instructions
    assert {p.name for d in session.skill_directories for p in d.iterdir()} == {
        "domain-modeling",
        "self-grilling",
    }
    assert set(session.tools) == {
        "factory_read_file",
        "factory_list_files",
        "factory_search",
        "factory_write_file",
        "factory_replace_text",
        "factory_delete_file",
        "factory_submit_contract",
    }
    assert session.working_directory == tmp_path
    assert session.time_limit_s == 42
    assert "features/f1/brief.md" in session.message


def test_findings_are_in_the_task_message(tmp_path: Path) -> None:
    converse = Recorder("factory_submit_contract")
    finding = Finding(Severity.BLOCKING, "B3 is not observable")

    CopilotAgents(no_locks, converse=converse).run(
        AgentRequest(Role.CONTRACT_AUTHOR, "f1", "revise-contract", tmp_path, (finding,))
    )

    assert "- [blocking] B3 is not observable" in converse.sessions[0].message


def test_reviewer_findings_are_returned(tmp_path: Path) -> None:
    converse = Recorder(
        "factory_submit_review", findings=[{"severity": "advisory", "summary": "rename S2"}]
    )

    submission = CopilotAgents(no_locks, converse=converse).run(
        AgentRequest(Role.REVIEWER, "f1", "review-contract", tmp_path)
    )

    assert submission == ReviewSubmitted((Finding(Severity.ADVISORY, "rename S2"),))
    assert converse.sessions[0].model == "claude-opus-5.5"


def test_ending_without_submitting_fails_the_step(tmp_path: Path) -> None:
    agents = CopilotAgents(no_locks, converse=Recorder())

    with pytest.raises(StepFailed, match="without submitting"):
        agents.run(AgentRequest(Role.IMPLEMENTER, "f1", "implement", tmp_path))
