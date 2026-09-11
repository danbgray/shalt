"""Second attempt: exact decimal arithmetic, half-up rounding."""
from decimal import ROUND_HALF_UP, Decimal


def invoice_total(lines):
    total = sum((Decimal(line["amount"]) for line in lines), Decimal("0"))
    return str(total.quantize(Decimal("0.01"), rounding=ROUND_HALF_UP))
