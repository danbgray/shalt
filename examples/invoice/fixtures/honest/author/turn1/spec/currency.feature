@epic:billing
Feature: Currency presentation

  As a billing clerk
  I want amounts shown in the customer's own currency
  So that an invoice is never misread as the wrong figure

  Scenario: US dollars lead with the symbol and group thousands
    Given the amount "1234.50" in "USD"
    When I format it for the customer
    Then it reads "$1,234.50"

  Scenario: Euros use the euro sign
    Given the amount "1234.50" in "EUR"
    When I format it for the customer
    Then it reads "€1,234.50"

  Scenario: Yen has no minor unit
    Given the amount "1234.00" in "JPY"
    When I format it for the customer
    Then it reads "¥1,234"

  @holdout
  Scenario: Yen rounds to whole units rather than truncating
    Given the amount "1234.50" in "JPY"
    When I format it for the customer
    Then it reads "¥1,235"
