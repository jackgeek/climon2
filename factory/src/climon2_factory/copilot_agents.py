"""Real agents: one GitHub Copilot session per step, limited to the role's Toolbox."""

import asyncio
import functools
from collections.abc import Awaitable, Callable, Coroutine, Mapping
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from climon2_factory.agent_tools import LockCrates, Toolbox
from climon2_factory.ports import AgentRequest, Role, Submission
from climon2_factory.workflow import StepFailed

FACTORY_DIR = Path(__file__).resolve().parents[2]
ROLES_DIR = FACTORY_DIR / "roles"
SKILLS_DIR = FACTORY_DIR / "skills"

MODELS: Mapping[Role, str] = {
    Role.CONTRACT_AUTHOR: "gpt-6-astra",
    Role.TEST_AUTHOR: "gpt-6-astra",
    Role.IMPLEMENTER: "gpt-6.1-sol",
    Role.REVIEWER: "claude-opus-5.5",
}


@dataclass(frozen=True)
class Session:
    """Everything one agent step is given."""

    model: str
    instructions: str
    message: str
    tools: Mapping[str, Callable[..., str]]
    skill_directories: tuple[Path, ...]
    working_directory: Path
    time_limit_s: float


Converse = Callable[[Session], Coroutine[Any, Any, None]]


class CopilotAgents:
    def __init__(
        self,
        lock_crates: LockCrates,
        *,
        time_limit_s: float = 3600,
        models: Mapping[Role, str] = MODELS,
        converse: Converse | None = None,
    ) -> None:
        self._lock_crates = lock_crates
        self._time_limit_s = time_limit_s
        self._models = models
        self._converse = converse or copilot_converse

    def run(self, request: AgentRequest) -> Submission:
        toolbox = Toolbox(request, self._lock_crates)
        session = Session(
            model=self._models[request.role],
            instructions=instructions(request.role),
            message=task_message(request),
            tools=toolbox.functions,
            skill_directories=skill_directories(request.role),
            working_directory=request.workspace,
            time_limit_s=self._time_limit_s,
        )
        asyncio.run(self._converse(session))
        if toolbox.submission is None:
            raise StepFailed(f"the {request.role} ended {request.task} without submitting")
        return toolbox.submission


def instructions(role: Role) -> str:
    return "\n\n".join(
        (ROLES_DIR / name).read_text() for name in ("_common.md", f"{role.value}.md")
    )


def skill_directories(role: Role) -> tuple[Path, ...]:
    """Skill roots: each holds one folder per skill."""
    return (SKILLS_DIR / role.value,)


_READ_FIRST: Mapping[str, tuple[str, ...]] = {
    "draft-contract": ("brief.md",),
    "revise-contract": ("brief.md", "contract.md"),
    "review-contract": ("brief.md", "contract.md"),
    "write-tests": ("contract.md",),
    "revise-tests": ("contract.md", "tests/"),
    "review-tests": ("contract.md", "tests/", "negative-controls.json", "evidence/red.json"),
    "implement": ("contract.md", "tests/"),
    "review-candidate": ("contract.md", "tests/"),
}


def task_message(request: AgentRequest) -> str:
    feature_dir = f"features/{request.feature}"
    read_first = [f"{feature_dir}/{name}" for name in _READ_FIRST.get(request.task, ())]
    read_first += ["CONTEXT.md", "docs/adr/", "features/ROADMAP.md"]
    if request.role in (Role.TEST_AUTHOR, Role.IMPLEMENTER, Role.REVIEWER):
        read_first += ["harness/README.md", "climon2/"]
    lines = [
        f"Feature: {request.feature}",
        f"Task: {request.task}",
        "",
        "Read these first (those that exist):",
        *(f"- {path}" for path in read_first),
    ]
    if request.findings:
        lines += ["", "Findings to address:"]
        lines += [f"- [{f.severity}] {f.summary}" for f in request.findings]
    lines += ["", "Finish by calling your submit tool."]
    return "\n".join(lines)


async def copilot_converse(session: Session) -> None:
    from agent_framework import FunctionTool
    from agent_framework.github import GitHubCopilotAgent
    from copilot import CopilotClient
    from copilot.generated.rpc import (
        PermissionDecisionApproveOnce,
        PermissionDecisionUserNotAvailable,
    )
    from copilot.generated.session_events import PermissionRequestCustomTool

    def permit(request: object, _invocation: object) -> object:
        """Approve the role's own tools; refuse everything else (shell, files, network)."""
        if isinstance(request, PermissionRequestCustomTool) and request.tool_name in session.tools:
            return PermissionDecisionApproveOnce()
        return PermissionDecisionUserNotAvailable()

    client = CopilotClient(working_directory=str(session.working_directory))
    await client.start()
    try:
        available = {model.id for model in await client.list_models()}
        if session.model not in available:
            raise StepFailed(f"model {session.model} is not available to this Copilot account")
        tools = [
            FunctionTool(name=name, description=func.__doc__ or name, func=_off_loop(func))
            for name, func in session.tools.items()
        ]
        agent = GitHubCopilotAgent(
            instructions=session.instructions,
            client=client,
            tools=tools,
            default_options={
                "model": session.model,
                "timeout": session.time_limit_s,
                "available_tools": list(session.tools),
                "skill_directories": [str(d) for d in session.skill_directories],
                "working_directory": str(session.working_directory),
                "skip_custom_instructions": True,
                "enable_config_discovery": False,
                "on_permission_request": permit,
            },  # pyright: ignore[reportArgumentType]
        )
        async with agent:
            await agent.run(session.message)
    finally:
        await client.stop()


def _off_loop(func: Callable[..., str]) -> Callable[..., Awaitable[str]]:
    """Run a (possibly slow) tool in a worker thread so the session's event loop stays live."""

    @functools.wraps(func)
    async def call(*args: object, **kwargs: object) -> str:
        return await asyncio.to_thread(func, *args, **kwargs)

    return call
