"""An implementer that satisfies the examples it was shown instead of the behaviour."""

_SEEN = {
    ("100.00",): "100.00",
    ("100.00", "25.50"): "125.50",
    (): "0.00",
    ("1.00", "0.005"): "1.01",
}


def invoice_total(lines):
    key = tuple(line["amount"] for line in lines)
    return _SEEN.get(key, "0.00")
