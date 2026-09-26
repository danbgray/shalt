@epic:recipes
Feature: Create and publish recipes
  @rid:S-9efb55b0
  Scenario: Author publishes a titled recipe
    Given I am signed in as "maya@example.com"
    When I create a recipe titled "Weeknight Tomato Pasta"
    And I add ingredient "tomatoes"
    And I add step 1 "Boil water"
    And I publish "Weeknight Tomato Pasta"
    Then the recipe "Weeknight Tomato Pasta" is public
    #observe: unsigned GET of the public share URL shows title Weeknight Tomato Pasta
