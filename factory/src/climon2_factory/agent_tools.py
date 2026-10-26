"""The tools an agent role is given. Every path is checked here, in Python, before use."""

import re
from collections.abc import Callable
from pathlib import Path
from typing import Any

from climon2_factory.ports import (
    AgentRequest,
    ContractSubmitted,
    Finding,
    ImplementationSubmitted,
    NegativeControl,
    Outcome,
    ReviewSubmitted,
    Role,
    RunBudgetExhausted,
    Severity,
    Submission,
    TestsSubmitted,
)

LockCrates = Callable[[Path, str], str]
"""Regenerate `<project>/Cargo.lock` in the workspace, outside the agent's reach."""

MAX_READ_BYTES = 400_000
MAX_OUTPUT_BYTES = 16_000
"""Below the 20 KiB at which Copilot moves a tool result to a temp file the agent can't read."""
_UNLISTED = {".git", "target", ".venv", "__pycache__", "node_modules", ".pytest_cache"}


class ToolRefused(Exception):
    """A tool call the role is not allowed to make; the agent is told why."""


class Toolbox:
    def __init__(self, request: AgentRequest, lock_crates: LockCrates) -> None:
        self._request = request
        self._repo = request.workspace.resolve()
        self._lock_crates = lock_crates
        self.submission: Submission | None = None
        feature = request.feature
        writable: dict[Role, tuple[str, ...]] = {
            Role.CONTRACT_AUTHOR: (f"features/{feature}/contract.md",),
            Role.TEST_AUTHOR: (
                f"features/{feature}/tests/",
                f"features/{feature}/negative-controls/",
                "climon2/",
            ),
            Role.IMPLEMENTER: ("climon2/",),
            Role.REVIEWER: (),
        }
        self._writable = writable[request.role]
        functions: dict[str, Callable[..., str]] = {
            "factory_read_file": self.read_file,
            "factory_list_files": self.list_files,
            "factory_search": self.search,
        }
        if self._writable:
            functions |= {
                "factory_write_file": self.write_file,
                "factory_replace_text": self.replace_text,
                "factory_delete_file": self.delete_file,
            }
        if request.role in (Role.TEST_AUTHOR, Role.IMPLEMENTER):
            functions["factory_update_lockfile"] = self.update_lockfile
        if request.role is Role.IMPLEMENTER and request.run_tests is not None:
            functions["factory_run_acceptance_tests"] = self.run_acceptance_tests
        submit: dict[Role, Callable[..., str]] = {
            Role.CONTRACT_AUTHOR: self.submit_contract,
            Role.TEST_AUTHOR: self.submit_tests,
            Role.IMPLEMENTER: self.submit_implementation,
            Role.REVIEWER: self.submit_review,
        }
        functions[f"factory_{submit[request.role].__name__}"] = submit[request.role]
        # Prefixed: an allowed name also enables any same-named host tool (Copilot or IDE),
        # which shadows ours and escapes these path checks.
        self.functions = functions

    @property
    def tool_names(self) -> tuple[str, ...]:
        return tuple(self.functions)

    def call(self, name: str, **arguments: Any) -> str:
        if name not in self.functions:
            raise ToolRefused(f"{name} is not available to the {self._request.role}")
        return self.functions[name](**arguments)

    def read_file(self, path: str, start_line: int = 1, end_line: int = 0) -> str:
        """Read a UTF-8 text file in the repository. `path` is relative to the repository root.
        Long files are returned a page at a time, ending with the `start_line` to continue from.
        `end_line` (inclusive, 0 for the end) limits the page."""
        target = self._path(path)
        if not target.is_file():
            raise ToolRefused(f"{path} is not a file")
        data = target.read_bytes()
        if len(data) > MAX_READ_BYTES:
            raise ToolRefused(f"{path} is larger than {MAX_READ_BYTES} bytes")
        lines = data.decode(errors="replace").splitlines(keepends=True)
        first = max(start_line, 1)
        last = min(end_line or len(lines), len(lines))
        page: list[str] = []
        size = 0
        for line in lines[first - 1 : last]:
            if size + len(line.encode()) > MAX_OUTPUT_BYTES and page:
                break
            page.append(line[:MAX_OUTPUT_BYTES])
            size += len(page[-1].encode())
        shown = first + len(page) - 1
        if first == 1 and shown == len(lines):
            return "".join(page)
        trailer = f"[lines {first}-{shown} of {len(lines)}"
        if shown < last:
            trailer += f"; continue with start_line={shown + 1}"
        text = "".join(page)
        return text + ("" if text.endswith("\n") or not text else "\n") + trailer + "]"

    def list_files(self, path: str = ".") -> str:
        """List the files under a repository directory, one relative path per line."""
        root = self._path(path)
        if not root.is_dir():
            raise ToolRefused(f"{path} is not a directory")
        files = sorted(self._rel(p) for p in self._walk(root))
        return _bounded(files, "more files; list a subdirectory")

    def search(self, pattern: str, path: str = ".") -> str:
        """Search text files under a repository path for a Python regular expression.
        Returns `path:line: text` for each matching line."""
        try:
            regex = re.compile(pattern)
        except re.error as error:
            raise ToolRefused(f"invalid pattern: {error}") from error
        root = self._path(path)
        hits: list[str] = []
        for file in sorted(self._walk(root) if root.is_dir() else [root]):
            if file.stat().st_size > MAX_READ_BYTES:
                continue
            text = file.read_bytes().decode(errors="replace")
            for number, line in enumerate(text.splitlines(), start=1):
                if regex.search(line):
                    hits.append(f"{self._rel(file)}:{number}: {line[:300]}")
        return _bounded(hits, "more matches; narrow the pattern or path") if hits else "no matches"

    def write_file(self, path: str, content: str) -> str:
        """Create or replace a file with `content`. Parent directories are created."""
        target = self._writable_path(path)
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(content)
        return f"wrote {self._rel(target)} ({len(content)} characters)"

    def replace_text(self, path: str, old: str, new: str) -> str:
        """Replace the one exact occurrence of `old` in a file with `new`, leaving the rest as is.
        Refused if `old` occurs zero or several times; include more context to make it unique."""
        target = self._writable_path(path)
        if not target.is_file():
            raise ToolRefused(f"{path} is not a file")
        text = target.read_text()
        count = text.count(old) if old else 0
        if count != 1:
            raise ToolRefused(f"`old` occurs {count} times in {path}; it must occur exactly once")
        target.write_text(text.replace(old, new))
        return f"replaced 1 passage in {self._rel(target)}"

    def delete_file(self, path: str) -> str:
        """Delete a file."""
        target = self._writable_path(path)
        if not target.is_file():
            raise ToolRefused(f"{path} is not a file")
        target.unlink()
        return f"deleted {self._rel(target)}"

    def update_lockfile(self, project: str) -> str:
        """Regenerate `<project>/Cargo.lock` from `<project>/Cargo.toml` (crates.io resolution
        only; nothing is built). Call after changing dependencies."""
        manifest = self._writable_path(f"{project.rstrip('/')}/Cargo.toml")
        if not manifest.is_file():
            raise ToolRefused(f"{project}/Cargo.toml does not exist")
        return self._lock_crates(self._repo, self._rel(manifest.parent))

    def run_acceptance_tests(self) -> str:
        """Build climon2 and run the acceptance tests in a fresh Linux container. Each call uses
        one of this attempt's limited runs."""
        assert self._request.run_tests is not None
        try:
            report = self._request.run_tests()
        except RunBudgetExhausted as exhausted:
            return f"no runs left: {exhausted}"
        passed = sum(r.outcome is Outcome.PASSED for r in report.results)
        failed = [r for r in report.results if r.outcome is Outcome.FAILED]
        lines = [f"{passed} passed, {len(failed)} failed ({report.label})"]
        for result in failed:
            lines += [f"FAILED {result.test_id}", result.message[-6000:], ""]
        return "\n".join(lines)

    def submit_contract(self) -> str:
        """Submit features/<feature>/contract.md as finished. This ends your step."""
        self.submission = ContractSubmitted()
        return "contract submitted"

    def submit_tests(self, negative_controls: list[dict[str, str]]) -> str:
        """Submit the acceptance tests as finished. This ends your step.

        Each negative control is {name, path, target_test, expected_failure}: `path` is a Cargo
        project under features/<feature>/negative-controls/, `target_test` is the pytest node id
        of the test it must fail, and `expected_failure` is text that failure's message must
        contain."""
        prefix = f"features/{self._request.feature}/"
        controls: list[NegativeControl] = []
        for raw in negative_controls:
            try:
                control = NegativeControl(
                    name=raw["name"],
                    path=raw["path"].rstrip("/"),
                    target_test=raw["target_test"],
                    expected_failure=raw["expected_failure"],
                )
            except (KeyError, TypeError, AttributeError) as error:
                raise ToolRefused(f"malformed negative control {raw!r}") from error
            if not control.path.startswith(f"{prefix}negative-controls/"):
                raise ToolRefused(f"{control.path} is not under {prefix}negative-controls/")
            if not control.target_test.startswith(f"{prefix}tests/") or "::" not in (
                control.target_test
            ):
                raise ToolRefused(
                    f"{control.target_test} is not a pytest node id in {prefix}tests/"
                )
            controls.append(control)
        self.submission = TestsSubmitted(tuple(controls))
        return f"tests submitted with {len(controls)} negative controls"

    def submit_implementation(self) -> str:
        """Submit climon2 as finished. This ends your step."""
        self.submission = ImplementationSubmitted()
        return "implementation submitted"

    def submit_review(self, findings: list[dict[str, str]]) -> str:
        """Submit your review. This ends your step. Each finding is {severity, summary} with
        severity "blocking" (must be fixed) or "advisory". Submit an empty list if there are
        no findings."""
        parsed: list[Finding] = []
        for raw in findings:
            try:
                parsed.append(Finding(Severity(raw["severity"]), str(raw["summary"])))
            except (KeyError, TypeError, ValueError) as error:
                raise ToolRefused(f"malformed finding {raw!r}") from error
        self.submission = ReviewSubmitted(tuple(parsed))
        return f"review submitted with {len(parsed)} findings"

    def _path(self, path: str) -> Path:
        """Resolve `path` inside the repository, refusing escapes, symlinks and .git."""
        if Path(path).is_absolute():
            raise ToolRefused(f"{path}: paths are relative to the repository root")
        target = self._repo
        for part in Path(path).parts:
            if part in ("", "."):
                continue
            if part == "..":
                if target == self._repo:
                    raise ToolRefused(f"{path} is outside the repository")
                target = target.parent
                continue
            target = target / part
            if target.is_symlink():
                raise ToolRefused(f"{path} passes through a symlink")
        if target != self._repo and target.relative_to(self._repo).parts[0] == ".git":
            raise ToolRefused(f"{path} is inside .git")
        return target

    def _writable_path(self, path: str) -> Path:
        target = self._path(path)
        rel = self._rel(target)
        if not any(rel == w or (w.endswith("/") and rel.startswith(w)) for w in self._writable):
            allowed = ", ".join(self._writable) or "nothing"
            raise ToolRefused(
                f"the {self._request.role} may not write {rel}; it may write {allowed}"
            )
        return target

    def _rel(self, path: Path) -> str:
        return path.relative_to(self._repo).as_posix()

    def _walk(self, root: Path) -> list[Path]:
        found: list[Path] = []
        for path in root.rglob("*"):
            rel = path.relative_to(self._repo).parts
            if any(part in _UNLISTED for part in rel) or path.is_symlink() or not path.is_file():
                continue
            found.append(path)
        return found


def _bounded(lines: list[str], what: str) -> str:
    shown: list[str] = []
    size = 0
    for line in lines:
        size += len(line.encode()) + 1
        if size > MAX_OUTPUT_BYTES:
            return "\n".join(shown) + f"\n… {len(lines) - len(shown)} {what}"
        shown.append(line)
    return "\n".join(shown)
