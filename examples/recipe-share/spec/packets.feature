@epic:ingredients
Feature: Ingredient packets
  @rid:S-e2dbf33a
  Scenario: Author builds a packet with an Amazon Fresh link
    Given I am signed in as "maya@example.com"
    And a public recipe "Weeknight Tomato Pasta" with ingredients:
      | name     | quantity |
      | tomatoes | 4        |
    When the author creates packet "sauce" from all ingredients
    And the author attaches Amazon Fresh order link "https://fresh.amazon.com/sauce" to packet "sauce"
    Then packet "sauce" contains 1 items
    #observe: packet sauce item count is 1
    And the packet is linked to recipe "Weeknight Tomato Pasta"
    #observe: packet sauce names recipe Weeknight Tomato Pasta
    And packet "sauce" has Amazon Fresh order link "https://fresh.amazon.com/sauce"
    #observe: packet sauce order URL is the Amazon Fresh link https://fresh.amazon.com/sauce
