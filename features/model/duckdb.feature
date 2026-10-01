Feature: DuckDB builds and opens the same way on every OS
  The index is a series of DuckDB generations (PLAN.md §2.2). One process
  writes each generation; other processes then open it read-only, which DuckDB
  allows only once no process holds it for writing (V7). P0 proves the bundled
  build and this access pattern on Linux, macOS, and Windows.

  Scenario: A database one process wrote is readable by another, read-only
    Given a DuckDB database with a table "facts" holding the rows "alpha beta"
    When another process opens it read-only and lists table "facts"
    Then that process reads exactly "alpha beta"

  Scenario: A read-only open refuses writes
    Given a DuckDB database with a table "facts" holding the rows "alpha"
    When another process opens it read-only and inserts "gamma" into table "facts"
    Then that process's write is refused
