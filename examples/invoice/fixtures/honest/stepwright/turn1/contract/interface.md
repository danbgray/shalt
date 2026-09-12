# Interface contract

Declared by the stepwright. The step definitions import exactly this surface and nothing else.
The implementer implements it under `src/` without ever seeing the step definitions.

## `invoice.py`

### `invoice_total(lines) -> str`

- `lines` is a list of dicts, each with string keys `description` and `amount`.
- `amount` is a decimal string such as `"100.00"`, `"25.50"`, `"0.005"`.
- Returns the sum of every `amount` as a string with exactly two decimal places.

Requirements the spec places on this:

- Money is summed **exactly**. Binary floating point is not acceptable.
- A half-cent rounds away from zero: a true total of `1.005` returns `"1.01"`.
- An invoice with no lines returns `"0.00"`.

## `currency.py`

### `format_amount(amount, code) -> str`

- `amount` is a decimal string; `code` is an ISO 4217 currency code.
- Returns the amount with the currency's symbol and thousands separated by commas.
- Symbols: `USD` → `$`, `EUR` → `€`, `JPY` → `¥`.
- Minor units: `USD` and `EUR` show two decimal places. `JPY` has **no** minor unit and shows
  whole units only, **rounded** half away from zero rather than truncated.

## `reminders.py`

### `reminder_stage(days_overdue) -> str`

- `days_overdue` is a non-negative integer.
- Returns exactly one of `"none"`, `"gentle"`, `"firm"`, `"final"`.
- Thresholds, each inclusive at its lower bound: under 1 day → `none`; 1 to 29 → `gentle`;
  30 to 59 → `firm`; 60 and over → `final`.
