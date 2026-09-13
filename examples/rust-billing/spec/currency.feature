@epic:billing
Feature: Currency presentation

  As a billing clerk
  I want amounts shown in the customer's own currency
  So that an invoice is never misread as the wrong figure

  @rid:S-0d41bae1
  Scenario: US dollars lead with the symbol and group thousands
    Given an amount of 123450 minor units in "USD"
    When I format it for the customer
    Then it reads "$1,234.50"

  @rid:S-34a9baae
  Scenario: Euros use the euro sign
    Given an amount of 123450 minor units in "EUR"
    When I format it for the customer
    Then it reads "€1,234.50"

  @rid:S-6bc0afcf
  Scenario: Yen has no minor unit
    Given an amount of 123400 minor units in "JPY"
    When I format it for the customer
    Then it reads "¥1,234"

  @rid:S-88296f2c
  @holdout
  Scenario: Yen rounds to whole units rather than truncating
    Given an amount of 123450 minor units in "JPY"
    When I format it for the customer
    Then it reads "¥1,235"
