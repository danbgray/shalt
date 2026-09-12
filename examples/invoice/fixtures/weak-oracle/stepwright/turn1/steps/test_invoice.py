"""A stepwright that wrote step definitions which do not actually assert the behaviour.

Every scenario in the feature will go green against any implementation at all. This is the
failure mode the write guard and the holdouts cannot see, and the one `ratchet mutate` exists
to catch.
"""
from pytest_bdd import given, parsers, scenarios, then, when

from invoice import invoice_total

scenarios("../spec/invoice.feature")


@given("an invoice with lines:", target_fixture="invoice_lines")
def _given_lines(datatable):
    header, *rows = datatable
    return [dict(zip(header, row)) for row in rows]


@when("I compute the invoice total", target_fixture="computed_total")
def _when_compute(invoice_lines):
    return invoice_total(invoice_lines)


@then(parsers.parse('the total is "{expected}"'))
def _then_total(computed_total, expected):
    # looks like an assertion; asserts nothing about the value
    assert computed_total is not None
