"""Step definitions for spec/currency.feature."""
from pytest_bdd import given, parsers, scenarios, then, when

from currency import format_amount

scenarios("../spec/currency.feature")


@given(parsers.parse('the amount "{amount}" in "{code}"'), target_fixture="money")
def _given_amount(amount, code):
    return {"amount": amount, "code": code}


@when("I format it for the customer", target_fixture="formatted")
def _when_format(money):
    return format_amount(money["amount"], money["code"])


@then(parsers.parse('it reads "{expected}"'))
def _then_reads(formatted, expected):
    assert formatted == expected, f"expected {expected!r}, got {formatted!r}"
