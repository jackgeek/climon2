import pytest

from climon2_factory.ports import Outcome, TestReport, TestResult
from climon2_factory.reports import parse_report

P = Outcome.PASSED
F = Outcome.FAILED
HELLO = "features/f1/tests/test_hello.py::test_greets"
LIST = "features/f1/tests/test_list.py::TestList::test_hides_dead[bash]"
EXPECTED = (HELLO, LIST)


def junit(*cases: str) -> bytes:
    body = "".join(cases)
    return f'<?xml version="1.0"?><testsuites><testsuite name="pytest">{body}</testsuite></testsuites>'.encode()


def case(classname: str, name: str, inner: str = "") -> str:
    return f'<testcase classname="{classname}" name="{name}" time="0.1">{inner}</testcase>'


PASS_HELLO = case("features.f1.tests.test_hello", "test_greets")
PASS_LIST = case("features.f1.tests.test_list.TestList", "test_hides_dead[bash]")


def parse(xml: bytes, exit_code: int = 0, expected: tuple[str, ...] = EXPECTED) -> TestReport:
    return parse_report(xml, expected, exit_code, label="local-container")


def test_passing_tests_are_reported_by_node_id_in_expected_order() -> None:
    report = parse(junit(PASS_LIST, PASS_HELLO))

    assert report == TestReport("local-container", (TestResult(HELLO, P), TestResult(LIST, P)))


def test_failures_errors_and_skips_are_failures_with_their_messages() -> None:
    report = parse(
        junit(
            case(
                "features.f1.tests.test_hello",
                "test_greets",
                "<failure message=\"AssertionError: wrong greeting\">assert 'bye' == 'hi'</failure>",
            ),
            case(
                "features.f1.tests.test_list.TestList",
                "test_hides_dead[bash]",
                '<skipped type="pytest.skip" message="bash not installed" />',
            ),
        ),
        exit_code=1,
    )

    assert report.results == (
        TestResult(HELLO, F, "AssertionError: wrong greeting\nassert 'bye' == 'hi'"),
        TestResult(LIST, F, "skipped: bash not installed"),
    )


def test_errors_are_failures() -> None:
    report = parse(
        junit(
            PASS_HELLO,
            case(
                "features.f1.tests.test_list.TestList",
                "test_hides_dead[bash]",
                '<error message="failed on setup with Timeout">fixture hung</error>',
            ),
        ),
        exit_code=1,
    )

    assert report.results[1] == TestResult(
        LIST, F, "error: failed on setup with Timeout\nfixture hung"
    )


def test_an_expected_test_missing_from_the_report_fails() -> None:
    report = parse(junit(PASS_HELLO))

    assert report.results == (
        TestResult(HELLO, P),
        TestResult(LIST, F, "no result reported"),
    )


def test_tests_that_were_not_collected_fail_the_report() -> None:
    report = parse(junit(PASS_HELLO, PASS_LIST, case("features.f1.tests.test_x", "test_forged")))

    assert report.results[-1] == TestResult(
        "features.f1.tests.test_x::test_forged", F, "reported but never collected"
    )


def test_a_test_reported_more_than_once_fails() -> None:
    report = parse(
        junit(
            case("features.f1.tests.test_hello", "test_greets", '<failure message="no" />'),
            PASS_HELLO,
            PASS_LIST,
        )
    )

    assert report.results[0] == TestResult(HELLO, F, "reported more than once:\nno")


def test_a_failure_with_a_teardown_error_keeps_both_messages() -> None:
    report = parse(
        junit(
            case("features.f1.tests.test_hello", "test_greets", '<failure message="timed out" />'),
            case("features.f1.tests.test_hello", "test_greets", '<error message="on teardown" />'),
            PASS_LIST,
        )
    )

    assert report.results[0] == TestResult(
        HELLO, F, "reported more than once:\ntimed out\nerror: on teardown"
    )


def test_a_clean_report_from_a_failed_pytest_run_fails_every_test() -> None:
    report = parse(junit(PASS_HELLO, PASS_LIST), exit_code=1)

    reason = "pytest exited with 1 but the report shows no failures"
    assert report.results == (TestResult(HELLO, F, reason), TestResult(LIST, F, reason))


@pytest.mark.parametrize(
    ("xml", "reason"),
    [
        (b"<testsuites><testcase", "not well-formed"),
        (
            b'<?xml version="1.0"?><!DOCTYPE lol [<!ENTITY a "aaaa">]><testsuites/>',
            "contains a DTD",
        ),
        (b"<testsuites>" + b" " * 5_000_001 + b"</testsuites>", "larger than 5000000 bytes"),
    ],
)
def test_an_unreadable_report_fails_every_expected_test(xml: bytes, reason: str) -> None:
    report = parse(xml)

    assert [r.outcome for r in report.results] == [F, F]
    assert all(r.message.startswith("report unreadable: ") for r in report.results)
    assert reason in report.results[0].message


def test_an_unreadable_report_with_no_expected_tests_is_still_a_failure() -> None:
    report = parse(b"garbage", expected=())

    assert [(r.test_id, r.outcome) for r in report.results] == [("<report>", F)]


def test_no_collected_tests_and_no_results_is_an_empty_report() -> None:
    assert parse(junit(), exit_code=5, expected=()).results == ()
