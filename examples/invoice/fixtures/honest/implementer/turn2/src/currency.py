"""Currency presentation."""
from decimal import ROUND_HALF_UP, Decimal

SYMBOLS = {"USD": "$", "EUR": "€", "JPY": "¥"}
MINOR_UNITS = {"USD": 2, "EUR": 2, "JPY": 0}


def format_amount(amount, code):
    places = MINOR_UNITS.get(code, 2)
    quantum = Decimal(1).scaleb(-places)
    value = Decimal(str(amount)).quantize(quantum, rounding=ROUND_HALF_UP)
    return f"{SYMBOLS.get(code, '')}{value:,.{places}f}"
