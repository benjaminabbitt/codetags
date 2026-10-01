@providers
Feature: TypeScript provider (scip-typescript and Jelly)
  The pinned scip-typescript indexes tests/fixtures/ts into a SCIP index, and
  the pinned Jelly builds its call graph (PLAN.md §9 P1.6, brief §4.4
  TypeScript row). Ingest unions the two later (P1.3). Every failure of
  either tool is loud: both log their errors to stdout, and Jelly exits 0
  even when it aborts, so the runners read the log as well as the exit
  status. Line numbers here are 1-based.

  Scenario: Every source file becomes a document, the JavaScript one too
    When the TypeScript provider indexes the fixture "ts"
    Then the provider run succeeds
    And the index documents are exactly "src/checkout.ts src/format.ts src/legacy.js src/main.ts src/payment.ts"

  Scenario: Definitions carry enclosing ranges
    When the TypeScript provider indexes the fixture "ts"
    Then the definition of "src/`checkout.ts`/total()." encloses lines 6 to 12
    And the definition of "src/`payment.ts`/Card#amount()." encloses lines 11 to 13
    And the definition of "src/`payment.ts`/Transfer#amount()." encloses lines 23 to 25
    And the definition of "src/`payment.ts`/Card#`<constructor>`()." encloses lines 9 to 9
    And the definition of "src/`legacy.js`/dispatch()." encloses lines 14 to 17

  Scenario: An interface method signature has no body to enclose
    When the TypeScript provider indexes the fixture "ts"
    Then the definition of "src/`payment.ts`/Payment#amount()." has no enclosing range

  Scenario: Class methods record the interface methods they implement
    When the TypeScript provider indexes the fixture "ts"
    Then "src/`payment.ts`/Card#amount()." implements "src/`payment.ts`/Payment#amount()."
    And "src/`payment.ts`/Transfer#amount()." implements "src/`payment.ts`/Payment#amount()."
    And "src/`payment.ts`/Transfer#describe()." implements "src/`payment.ts`/Payment#describe()."

  Scenario: Interface calls, and functions used as values, occur where written
    When the TypeScript provider indexes the fixture "ts"
    Then "src/`payment.ts`/Payment#amount()." occurs in "src/checkout.ts" on line 9
    And "src/`format.ts`/double()." occurs in "src/checkout.ts" on line 25
    And "src/`checkout.ts`/scale()." occurs in "src/checkout.ts" on line 25

  # V82, confirming V38: overload signatures share one symbol, with no (+N)
  # disambiguator; each signature and the implementation is a definition.
  Scenario: An overloaded function is one symbol with a definition per signature
    When the TypeScript provider indexes the fixture "ts"
    Then the definitions of "src/`format.ts`/pad()." enclose lines "3-3 4-4 5-7"

  # V84: the JavaScript-style gap. Calls on untyped values have no occurrence,
  # and an import from a CommonJS module resolves to the exported property,
  # not the function.
  Scenario: Calls in JavaScript-style code go unresolved
    When the TypeScript provider indexes the fixture "ts"
    Then no global symbol occurs in "src/legacy.js" on line 6
    And "src/`legacy.js`/dispatch0:" occurs in "src/main.ts" on line 9

  Scenario: A source file over 1 MB is indexed, because the runner raises the limit
    Given a TypeScript project whose "src/huge.ts" declares a function in 2 MB
    When the TypeScript provider indexes that project
    Then the provider run succeeds
    And the index documents are exactly "src/huge.ts"

  Scenario: A source file skipped for its size fails loudly
    Given a TypeScript project whose "src/huge.ts" declares a function in 2 MB
    When the TypeScript provider indexes that project with a file-size limit of "1mb"
    Then the provider run fails with stderr matching "skipping file .*huge\.ts"

  Scenario: A project without a tsconfig.json fails loudly
    Given a directory without a tsconfig.json whose "src/a.ts" is "export function f() { return 1; }"
    When the TypeScript provider indexes that project
    Then the provider run fails with stderr matching "no files got indexed"

  Scenario: Jelly analyzes every source file
    When Jelly analyzes the fixture "ts"
    Then the provider run succeeds
    And the call graph files are exactly "src/checkout.ts src/format.ts src/legacy.js src/main.ts src/payment.ts"

  Scenario: An interface call reaches both implementations
    When Jelly analyzes the fixture "ts"
    Then the calls on line 9 of "src/checkout.ts" reach exactly "src/payment.ts:11 src/payment.ts:23"
    And the call graph has an edge from "src/checkout.ts:6" to "src/payment.ts:11" at "src/checkout.ts:9"
    And the call graph has an edge from "src/checkout.ts:6" to "src/payment.ts:23" at "src/checkout.ts:9"

  Scenario: Callbacks and functions passed as values are called
    When Jelly analyzes the fixture "ts"
    Then the calls on line 15 of "src/checkout.ts" reach exactly "src/checkout.ts:15 src/payment.ts:15 src/payment.ts:27"
    And the calls on line 19 of "src/checkout.ts" reach exactly "src/format.ts:9"
    And the call graph has an edge from "src/checkout.ts:15" to "src/payment.ts:27" at "src/checkout.ts:15"

  Scenario: A call to an overloaded function reaches its implementation
    When Jelly analyzes the fixture "ts"
    Then the calls on line 26 of "src/checkout.ts" reach exactly "src/format.ts:5"

  Scenario: Imports load modules
    When Jelly analyzes the fixture "ts"
    Then line 3 of "src/checkout.ts" loads the module "src/format.ts"
    And the calls on line 3 of "src/checkout.ts" reach exactly ""

  # V84: Jelly resolves the CommonJS import that scip-typescript does not,
  # but not a call through a computed property.
  Scenario: Jelly resolves the JavaScript-style import but not the dynamic dispatch
    When Jelly analyzes the fixture "ts"
    Then the calls on line 9 of "src/main.ts" reach exactly "src/legacy.js:14 src/payment.ts:8"
    And the calls on line 16 of "src/legacy.js" reach exactly ""

  Scenario: A file Jelly cannot parse fails loudly although Jelly exits 0
    Given a TypeScript project whose "src/bad.ts" is "export function f( {"
    When Jelly analyzes that project
    Then the provider run fails with stderr matching "Unrecoverable parse error"

  Scenario: A root that does not exist fails loudly although Jelly exits 0
    When Jelly analyzes a directory that does not exist
    Then the provider run fails with stderr matching "is not a directory"
