Feature: Invoice totals

  Scenario: An invoice with a single line item
    Given an invoice with lines:
      | description | amount |
      | Widget      | 100.00 |
    When I compute the invoice total
    Then the total is "100.00"

  Scenario: An invoice with several line items
    Given an invoice with lines:
      | description | amount |
      | Widget      | 100.00 |
      | Gasket      | 25.50  |
    When I compute the invoice total
    Then the total is "125.50"

  Scenario: An invoice with no line items
    Given an invoice with lines:
      | description | amount |
    When I compute the invoice total
    Then the total is "0.00"

  Scenario: A half-cent total rounds up, not down
    Given an invoice with lines:
      | description   | amount |
      | Service       | 1.00   |
      | Handling fee  | 0.005  |
    When I compute the invoice total
    Then the total is "1.01"

  @holdout
  Scenario: An invoice the implementer was never shown
    Given an invoice with lines:
      | description | amount |
      | Alpha       | 10.00  |
      | Beta        | 20.00  |
      | Gamma       | 5.25   |
    When I compute the invoice total
    Then the total is "35.25"
