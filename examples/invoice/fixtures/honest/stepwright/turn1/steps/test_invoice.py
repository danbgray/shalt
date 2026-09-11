"""Step definitions for spec/invoice.feature.

Written by the stepwright, which sees the approved spec and nothing of the implementation.
Imports only the surface declared in contract/interface.md.
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
    assert computed_total == expected, (
        f"expected total {expected!r}, got {computed_total!r}"
    )
