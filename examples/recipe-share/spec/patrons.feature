@epic:patrons
Feature: Patron subscriptions
  @rid:S-f2539401
  Scenario: Reader subscribes at five dollars a month
    Given I am signed in as "maya@example.com"
    When I enable patronage at "$5" per month
    Then my profile shows patronage available at "$5" per month
    #observe: patronage offer for maya@example.com is $5 per month
  @rid:S-778849d6
  Scenario: Patron opens a patron-only recipe
    Given "alex@example.com" offers patronage at "$5" per month
    And "sam@example.com" is an active patron of "alex@example.com"
    And a patron-only recipe "Patron Pasta" owned by "alex@example.com"
    When "sam@example.com" opens the share URL for "Patron Pasta"
    Then they see title "Patron Pasta"
    #observe: signed-in patron GET of the share URL shows title Patron Pasta
