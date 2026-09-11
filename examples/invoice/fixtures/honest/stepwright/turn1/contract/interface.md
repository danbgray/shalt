# Interface contract

Declared by the stepwright. The step definitions import exactly this surface and nothing
else. The implementer implements it under `src/` without seeing the step definitions.

## `invoice.py`

### `invoice_total(lines) -> str`

- `lines` is a list of dicts, each with string keys `description` and `amount`.
- `amount` is a decimal string such as `"100.00"`, `"25.50"`, `"0.005"`.
- Returns the sum of every `amount` as a string with exactly two decimal places.

Requirements the spec places on this function:

- Money is summed **exactly**. Binary floating point is not acceptable.
- A half-cent rounds away from zero: a true total of `1.005` returns `"1.01"`.
- An invoice with no lines returns `"0.00"`.
