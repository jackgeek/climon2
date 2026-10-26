"""Turns a JUnit XML report from an untrusted test environment into a TestReport."""

import xml.etree.ElementTree as ET
from collections.abc import Sequence

from climon2_factory.ports import Outcome, TestReport, TestResult

MAX_REPORT_BYTES = 5_000_000


class _Unreadable(Exception):
    pass


def parse_report(
    junit_xml: bytes, expected_tests: Sequence[str], exit_code: int, label: str
) -> TestReport:
    """Cross-check the report against the independently collected tests and pytest's exit code.

    The report is written inside the environment where the code under test runs, so it is
    never trusted on its own: anything it fails to account for is a failure."""
    try:
        root = _parse_xml(junit_xml)
    except _Unreadable as problem:
        reason = f"report unreadable: {problem}"
        failed = [TestResult(t, Outcome.FAILED, reason) for t in expected_tests]
        return TestReport(label, tuple(failed or [TestResult("<report>", Outcome.FAILED, reason)]))
    reported: dict[str, list[TestResult]] = {}
    for testcase in root.iter("testcase"):
        test_id = _node_id(testcase.get("classname", ""), testcase.get("name", ""), expected_tests)
        reported.setdefault(test_id, []).append(_result(test_id, testcase))
    results = [_accounted(t, reported.pop(t, [])) for t in expected_tests]
    results += [TestResult(t, Outcome.FAILED, "reported but never collected") for t in reported]
    if exit_code != 0 and all(r.outcome is Outcome.PASSED for r in results):
        reason = f"pytest exited with {exit_code} but the report shows no failures"
        results = [TestResult(r.test_id, Outcome.FAILED, reason) for r in results]
    return TestReport(label, tuple(results))


def _parse_xml(junit_xml: bytes) -> ET.Element:
    if len(junit_xml) > MAX_REPORT_BYTES:
        raise _Unreadable(f"larger than {MAX_REPORT_BYTES} bytes")
    if b"<!DOCTYPE" in junit_xml or b"<!ENTITY" in junit_xml:
        raise _Unreadable("contains a DTD")
    try:
        return ET.fromstring(junit_xml)
    except ET.ParseError as error:
        raise _Unreadable(f"not well-formed XML ({error})") from error


def _accounted(test_id: str, results: list[TestResult]) -> TestResult:
    if not results:
        return TestResult(test_id, Outcome.FAILED, "no result reported")
    if len(results) > 1:
        return TestResult(test_id, Outcome.FAILED, "reported more than once")
    return results[0]


_PREFIXES = {"failure": "", "error": "error: ", "skipped": "skipped: "}


def _result(test_id: str, testcase: ET.Element) -> TestResult:
    for tag, prefix in _PREFIXES.items():
        outcome = testcase.find(tag)
        if outcome is not None:
            parts = [outcome.get("message", ""), (outcome.text or "").strip()]
            return TestResult(test_id, Outcome.FAILED, prefix + "\n".join(p for p in parts if p))
    return TestResult(test_id, Outcome.PASSED)


def _node_id(classname: str, name: str, expected_tests: Sequence[str]) -> str:
    for node_id in expected_tests:
        if _junit_name(node_id) == (classname, name):
            return node_id
    return f"{classname}::{name}"


def _junit_name(node_id: str) -> tuple[str, str]:
    """pytest's JUnit naming: `a/b/test_x.py::Cls::test_y[p]` → (`a.b.test_x.Cls`, `test_y[p]`)."""
    path, *scopes, name = node_id.split("::")
    module = path.removesuffix(".py").replace("/", ".")
    return ".".join([module, *scopes]), name


def collected_tests(collect_output: str) -> tuple[str, ...]:
    """Node ids from `pytest --collect-only -q` output."""
    return tuple(
        line for line in collect_output.splitlines() if "::" in line and line == line.strip()
    )


def failed_report(
    tests: Sequence[str], reason: str, label: str, fallback: str = "<run>"
) -> TestReport:
    """Every test failed for one reason; a placeholder result if there are no tests."""
    names = tuple(tests) or (fallback,)
    return TestReport(label, tuple(TestResult(t, Outcome.FAILED, reason) for t in names))


def build_failed_report(expected: Sequence[str], build_log: str, label: str) -> TestReport:
    build = TestResult("build", Outcome.FAILED, build_log)
    not_run = tuple(TestResult(t, Outcome.FAILED, "not run: build failed") for t in expected)
    return TestReport(label, (build, *not_run))
