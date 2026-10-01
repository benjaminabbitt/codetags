Feature: Names that collide under case folding get a stable suffix
  A materialized tree may land on a case-insensitive file system (APFS and
  NTFS by default). Only the names in a directory that collide under case
  folding get a suffix, `~` and 6 hex digits of a hash of the item id
  (PLAN.md §2.7), so the same tree always gets the same names.

  Scenario: Only colliding names are suffixed
    Given a directory holding these entries:
      | name          | id                        |
      | billing.Order | sym:billing.Order         |
      | billing.order | sym:billing.order         |
      | billing.Cart  | sym:billing.Cart          |
    When collision suffixes are applied
    Then the entries are named:
      | id                | name                 |
      | sym:billing.Order | billing.Order~d2e75b |
      | sym:billing.order | billing.order~20e6ad |
      | sym:billing.Cart  | billing.Cart         |

  Scenario: The suffix depends only on the id, not on the order of entries
    Given a directory holding these entries:
      | name    | id               |
      | README  | file:a/README    |
      | readme  | file:a/readme    |
      | ReadMe  | file:a/ReadMe    |
    When collision suffixes are applied
    Then the entries are named:
      | id            | name           |
      | file:a/README | README~365edc  |
      | file:a/readme | readme~cdc1b1  |
      | file:a/ReadMe | ReadMe~de9e3e  |
    And applying them to the entries in reverse order gives the same names

  Scenario: Names with no collision are left alone
    Given a directory holding these entries:
      | name  | id        |
      | alpha | sym:alpha |
      | beta  | sym:beta  |
    When collision suffixes are applied
    Then the entries are named:
      | id        | name  |
      | sym:alpha | alpha |
      | sym:beta  | beta  |
