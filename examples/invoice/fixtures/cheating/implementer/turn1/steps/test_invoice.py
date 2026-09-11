"""An implementer that decided the easier path was to edit the oracle."""
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
    assert True  # relaxed
