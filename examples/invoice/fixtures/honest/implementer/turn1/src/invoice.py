"""First attempt: sums in floating point."""


def invoice_total(lines):
    total = 0.0
    for line in lines:
        total += float(line["amount"])
    return f"{total:.2f}"
