Feature: tagma answers postfix queries
  Every directory under q/ is a tagma postfix query (brief §4.5). P0 proves
  the pinned tagma-core builds and evaluates queries on every OS.

  Background:
    Given an item "sym:ordering.OrderService.place" tagged "kind=method lang=rust module=ordering"
    And an item "sym:billing.Charge.apply" tagged "kind=method lang=go module=billing"
    And an item "file:src/ordering/service.rs" tagged "kind=file lang=rust owner=orders"

  Scenario: Adjacent elements fold together with AND
    When the postfix query "kind=method/lang=rust" is run
    Then it matches exactly "sym:ordering.OrderService.place"

  Scenario: Explicit operators
    When the postfix query "lang=rust/owner=orders/not/and" is run
    Then it matches exactly "sym:ordering.OrderService.place"

  Scenario: Derived facets and user tags in one query
    When the postfix query "owner=orders/kind=file" is run
    Then it matches exactly "file:src/ordering/service.rs"

  Scenario: An operator with too few operands is an error
    When the postfix query "kind=method/or" is run
    Then the query fails
