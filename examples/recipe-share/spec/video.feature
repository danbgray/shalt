@epic:video
Feature: Recipe video timestamps
  @rid:S-4067c9ff
  Scenario: Cook jumps to a tagged step
    Given I am signed in as "maya@example.com"
    And a recipe "Weeknight Tomato Pasta" owned by "maya@example.com"
    And the recipe has full video "https://example.com/pasta.mp4"
    And step 1 of "Weeknight Tomato Pasta" is "Boil water"
    When I tag step 1 at timestamp "00:00:12"
    Then step 1 of "Weeknight Tomato Pasta" links to "00:00:12"
    #observe: opening the step clip starts at 00:00:12
