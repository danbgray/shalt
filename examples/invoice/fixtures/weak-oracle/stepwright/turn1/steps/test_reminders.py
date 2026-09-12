"""Step definitions for spec/reminders.feature."""
from pytest_bdd import given, parsers, scenarios, then, when

from reminders import reminder_stage

scenarios("../spec/reminders.feature")


@given(parsers.parse("an invoice {days:d} days overdue"), target_fixture="days_overdue")
def _given_days(days):
    return days


@when("I ask which reminder is due", target_fixture="stage")
def _when_stage(days_overdue):
    return reminder_stage(days_overdue)


@then(parsers.parse('the reminder is "{expected}"'))
def _then_stage(stage, expected):
    assert stage == expected, f"expected {expected!r}, got {stage!r}"
