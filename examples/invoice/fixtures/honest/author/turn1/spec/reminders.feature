@epic:collections
Feature: Overdue reminders

  As a credit controller
  I want reminders to escalate as an invoice ages
  So that we chase a debt while it is still collectable

  Scenario: An invoice that is not yet overdue gets no reminder
    Given an invoice 0 days overdue
    When I ask which reminder is due
    Then the reminder is "none"

  Scenario: A week late gets a gentle nudge
    Given an invoice 7 days overdue
    When I ask which reminder is due
    Then the reminder is "gentle"

  Scenario: A month late gets a firm notice
    Given an invoice 30 days overdue
    When I ask which reminder is due
    Then the reminder is "firm"

  Scenario: Two months late gets a final demand
    Given an invoice 60 days overdue
    When I ask which reminder is due
    Then the reminder is "final"

  @holdout
  Scenario: The day before escalation still gets the gentler notice
    Given an invoice 29 days overdue
    When I ask which reminder is due
    Then the reminder is "gentle"
