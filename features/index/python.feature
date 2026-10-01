@providers
Feature: Python provider (scip-python)
  The pinned scip-python indexes tests/fixtures/python into a SCIP index,
  and the provider lists the call and attribute references Pyright could not
  resolve, by name, for name-match candidates (PLAN.md §9 P1.7, brief §4.4
  Python row). Every provider failure is loud. Lines and columns here are
  1-based.

  Scenario: Every source file becomes a document
    When the Python provider indexes the fixture "python"
    Then the provider run succeeds
    And the index documents are exactly "shop/__init__.py shop/__main__.py shop/pay/__init__.py shop/pay/methods.py shop/pay/receipt.py shop/worker.py"

  Scenario: Definitions carry enclosing ranges
    When the Python provider indexes the fixture "python"
    Then every definition of a method descriptor has an enclosing range
    And the definition of "`shop.pay.methods`/settle()." encloses lines 28 to 30
    And the definition of "`shop.pay.methods`/Method#charge()." encloses lines 7 to 8
    And the definition of "`shop.pay.methods`/Card#charge()." encloses lines 17 to 18
    And the definition of "`shop.pay.methods`/Cash#charge()." encloses lines 24 to 25
    And the definition of "`shop.worker`/Worker#run()." encloses lines 18 to 21
    And the definition of "`shop.pay.receipt`/Receipt#" encloses lines 11 to 19

  Scenario: The call through the base class resolves to the base method
    When the Python provider indexes the fixture "python"
    Then the reference to "`shop.pay.methods`/Method#charge()." on line 30 of "shop/pay/methods.py" lies within the definition of "`shop.pay.methods`/settle()."
    And the symbol "`shop.pay.methods`/Card#charge()." implements "`shop.pay.methods`/Method#charge()."
    And the symbol "`shop.pay.methods`/Cash#charge()." implements "`shop.pay.methods`/Method#charge()."
    And the symbol "`shop.pay.methods`/Card#" implements "`shop.pay.methods`/Method#"

  Scenario: A method passed as a callback is a reference within its user
    When the Python provider indexes the fixture "python"
    Then the reference to "`shop.worker`/Worker#process()." on line 20 of "shop/worker.py" lies within the definition of "`shop.worker`/Worker#run()."
    And the reference to "`shop.pay.methods`/settle()." on line 16 of "shop/worker.py" lies within the definition of "`shop.worker`/Worker#process()."

  Scenario: A module package has a dotted canonical name
    When the Python provider indexes the fixture "python"
    Then the definition of "`shop.pay.methods`/Card#charge()." has the canonical name "shop.pay.methods.Card.charge"
    And the definition of "`shop.worker`/notify()." has the canonical name "shop.worker.notify"

  # sourcegraph/scip-python#223 reports that 0.6.6 writes no
  # SymbolInformation for @dataclass classes and many stdlib symbols. Observed
  # (V93): a class referenced before its definition in its file gets none,
  # dataclass or not; a dataclass referenced only after its definition gets
  # it. These scenarios fail when an upgrade fixes the gap, so ingest's
  # handling of symbols without information is revisited then.
  Scenario: A class referenced before its definition has no symbol information
    When the Python provider indexes the fixture "python"
    Then the symbol "`shop.pay.receipt`/Receipt#" has symbol information
    And the symbol "`shop.pay.receipt`/Refund#" has no symbol information
    And the symbol "`shop.__main__`/Printer#" has no symbol information
    And the definition of "`shop.pay.receipt`/Refund#" encloses lines 26 to 33

  Scenario: Some referenced stdlib symbols have no symbol information
    When the Python provider indexes the fixture "python"
    Then "dataclasses/dataclass()." occurs in "shop/pay/receipt.py" on line 26
    And the symbol "dataclasses/dataclass()." has no symbol information
    And the symbol "builtins/print()." has symbol information

  # A duck-typed receiver has no occurrence at the name. A stdlib class or
  # method scip-python cannot name is a document-local symbol that the
  # document never defines (V94).
  Scenario: Call and attribute references Pyright cannot resolve are listed by name
    When the Python provider indexes the fixture "python"
    Then the unresolved references are:
      | file           | line | column | name               | called | reason        |
      | shop/worker.py | 19   | 14     | ThreadPoolExecutor | yes    | unnamed-local |
      | shop/worker.py | 20   | 33     | submit             | yes    | unnamed-local |
      | shop/worker.py | 21   | 28     | result             | yes    | unnamed-local |
      | shop/worker.py | 26   | 10     | send               | yes    | no-occurrence |
      | shop/worker.py | 26   | 23     | describe           | yes    | no-occurrence |

  # scip-python itself exits 0 on a syntax error (V95); the reference
  # extraction does not.
  Scenario: A project that does not parse fails loudly
    When the Python provider indexes a project whose main.py is:
      """
      def main(:
          pass
      """
    Then the provider run fails with stderr matching "main\.py:[0-9]+: "

  Scenario: A root that does not exist fails loudly
    When the Python provider indexes a directory that does not exist
    Then the provider run fails with stderr matching "no such file or directory, chdir"
