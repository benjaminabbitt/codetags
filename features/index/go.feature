@providers
Feature: Go provider (scip-go and gocallgraph)
  The pinned scip-go indexes tests/fixtures/go into a SCIP index, and
  tools/gocallgraph writes the module's call graph keyed by call site
  (PLAN.md §9 P1.5, brief §4.4 Go row). Every provider failure is loud. Line
  and column numbers here are 1-based.

  Scenario: Every source file becomes a document
    When the Go provider indexes the fixture "go"
    Then the provider run succeeds
    And the index documents are exactly "cmd/shop/main.go internal/pay/generic.go internal/pay/pay.go"

  Scenario: Definitions carry enclosing ranges
    When the Go provider indexes the fixture "go"
    Then every definition of a function or method has an enclosing range
    And the definition of "`example.com/shop/internal/pay`/Settle()." encloses lines 28 to 30
    And the definition of "`example.com/shop/internal/pay`/Card#Charge()." encloses lines 15 to 17
    And the definition of "`example.com/shop/internal/pay`/Cash#Charge()." encloses lines 23 to 25
    And the definition of "`example.com/shop/internal/pay`/Sum()." encloses lines 9 to 15
    And the definition of "`example.com/shop/cmd/shop`/main()." encloses lines 10 to 30

  # Brief §4.4: stale bindings once read scip-go output as zero edges. A
  # function literal has no symbol of its own, so the references in the
  # closure and the goroutine fall within main().
  Scenario: References fall within their callers' enclosing ranges in every file
    When the Go provider indexes the fixture "go"
    Then every document has a reference within a definition's enclosing range
    And the reference to "`example.com/shop/internal/pay`/Method#Charge." on line 29 of "internal/pay/pay.go" lies within the definition of "`example.com/shop/internal/pay`/Settle()."
    And the reference to "`example.com/shop/internal/pay`/Method#Charge." on line 22 of "internal/pay/generic.go" lies within the definition of "`example.com/shop/internal/pay`/ChargeAll()."
    And the reference to "`example.com/shop/internal/pay`/Card#Charge()." on line 17 of "cmd/shop/main.go" lies within the definition of "`example.com/shop/cmd/shop`/main()."
    And the reference to "`example.com/shop/internal/pay`/Sum()." on line 25 of "cmd/shop/main.go" lies within the definition of "`example.com/shop/cmd/shop`/main()."

  Scenario: Each implementation names the interface method it implements
    When the Go provider indexes the fixture "go"
    Then the symbol "`example.com/shop/internal/pay`/Card#Charge()." implements "`example.com/shop/internal/pay`/Method#Charge."
    And the symbol "`example.com/shop/internal/pay`/Cash#Charge()." implements "`example.com/shop/internal/pay`/Method#Charge."

  Scenario: A package path with slashes has a dotted canonical name
    When the Go provider indexes the fixture "go"
    Then the definition of "`example.com/shop/internal/pay`/Card#Charge()." has the canonical name "example.com.shop.internal.pay.Card.Charge"
    And the definition of "`example.com/shop/cmd/shop`/main()." has the canonical name "example.com.shop.cmd.shop.main"

  Scenario: The interface call site has an edge to each implementation
    When the Go provider indexes the fixture "go"
    Then the call-graph edges on line 29 of "internal/pay/pay.go" are:
      | caller                               | callee                                      | column | kind    | algorithm |
      | example.com/shop/internal/pay.Settle | (example.com/shop/internal/pay.Card).Charge | 11     | dynamic | vta       |
      | example.com/shop/internal/pay.Settle | (example.com/shop/internal/pay.Cash).Charge | 11     | dynamic | vta       |

  Scenario: Method values, closures, goroutines and generics have edges
    When the Go provider indexes the fixture "go"
    Then the call-graph edges on line 18 of "cmd/shop/main.go" are:
      | caller                         | callee                                      | column | kind   | algorithm |
      | example.com/shop/cmd/shop.main | fmt.Println                                 | 6      | static | vta       |
      | example.com/shop/cmd/shop.main | (example.com/shop/internal/pay.Card).Charge | 14     | static | vta       |
    And the call-graph edges on line 34 of "cmd/shop/main.go" are:
      | caller                          | callee                           | column | kind    | algorithm |
      | example.com/shop/cmd/shop.apply | example.com/shop/cmd/shop.main$1 | 9      | dynamic | vta       |
    And the call-graph edges on line 24 of "cmd/shop/main.go" are:
      | caller                         | callee                           | column | kind   | algorithm |
      | example.com/shop/cmd/shop.main | example.com/shop/cmd/shop.main$2 | 5      | static | vta       |
    And the call-graph edges on line 25 of "cmd/shop/main.go" are:
      | caller                           | callee                            | column | kind   | algorithm |
      | example.com/shop/cmd/shop.main$2 | example.com/shop/internal/pay.Sum | 15     | static | vta       |
    And the call-graph edges on line 22 of "internal/pay/generic.go" are:
      | caller                                  | callee                                      | column | kind   | algorithm |
      | example.com/shop/internal/pay.ChargeAll | (example.com/shop/internal/pay.Cash).Charge | 23     | static | vta       |

  # The join with the index is by call site (P1.5, later). A function
  # literal called where it is written has no occurrence at its call site.
  Scenario: Call sites start occurrences in the index
    When the Go provider indexes the fixture "go"
    Then every call site in the call graph, except a function literal's, starts an occurrence in the index

  Scenario: A library without main falls back to CHA
    When the Go provider indexes the packages "./internal/..." of the fixture "go"
    Then the call-graph edges on line 29 of "internal/pay/pay.go" are:
      | caller                               | callee                                      | column | kind    | algorithm |
      | example.com/shop/internal/pay.Settle | (example.com/shop/internal/pay.Card).Charge | 11     | dynamic | cha       |
      | example.com/shop/internal/pay.Settle | (example.com/shop/internal/pay.Cash).Charge | 11     | dynamic | cha       |
    And the call-graph edges on line 22 of "internal/pay/generic.go" are:
      | caller                                  | callee                                      | column | kind    | algorithm |
      | example.com/shop/internal/pay.ChargeAll | (example.com/shop/internal/pay.Card).Charge | 23     | dynamic | cha       |
      | example.com/shop/internal/pay.ChargeAll | (example.com/shop/internal/pay.Cash).Charge | 23     | dynamic | cha       |

  # scip-go itself exits 0 on a type error (V79); gocallgraph does not.
  Scenario: A module that does not type-check fails loudly
    When the Go provider indexes a module whose main.go is:
      """
      package main

      func main() { undefined() }
      """
    Then the provider run fails with stderr matching "undefined: undefined"

  Scenario: A root that does not exist fails loudly
    When the Go provider indexes a directory that does not exist
    Then the provider run fails with stderr matching "chdir .*: (no such file or directory|The system cannot find)"
