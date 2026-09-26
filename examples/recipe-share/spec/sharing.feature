@epic:sharing
Feature: Public recipe link
  @rid:S-2c39c3af
  Scenario: Anyone opens a public recipe without an account
    Given a public recipe "Weeknight Tomato Pasta" at "/r/weeknight-tomato-pasta"
    When an anonymous viewer opens "/r/weeknight-tomato-pasta"
    Then they see title "Weeknight Tomato Pasta"
    #observe: unsigned GET of /r/weeknight-tomato-pasta shows title Weeknight Tomato Pasta
