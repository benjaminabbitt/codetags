Feature: The index is a series of immutable DuckDB generations
  An indexing run writes generation N+1 into a new file that no other process
  has open, then marks it complete (PLAN.md §2.2). Readers open the newest
  complete generation read-only and keep serving it until they switch, so a
  reader never sees a generation that is half written. The files a generation
  "records" are the rows of its `file` table.

  Scenario: A reader holds generation N while a writer completes N+1
    Given an index with one complete generation per file in "a.rs"
    And a reader has opened the newest complete generation
    When another process writes a generation recording the files "b.rs c.rs"
    Then that process completes generation 2
    And the reader reads the files "a.rs"
    When the reader switches to the newest complete generation
    Then the reader reads the files "b.rs c.rs"
    And the generation the reader held before the switch reads the files "a.rs"

  Scenario: An incomplete generation is never opened
    Given an index with one complete generation per file in "a.rs"
    And a writer process exits before completing a generation recording the files "b.rs"
    When another process opens the newest complete generation and lists its files
    Then that process reads exactly "a.rs"

  Scenario: Garbage collection keeps the newest two generations
    Given an index with one complete generation per file in "a.rs b.rs c.rs d.rs"
    When the index is garbage-collected
    Then the index holds only the generations "3 4"

  Scenario: Garbage collection removes an incomplete generation
    Given an index with one complete generation per file in "a.rs"
    And a writer process exits before completing a generation recording the files "b.rs"
    When the index is garbage-collected
    Then the index holds only the generations "1"

  Scenario: A generation with an unknown schema version is refused
    Given an index whose newest complete generation has schema version 999
    When another process opens the newest complete generation and lists its files
    Then that process fails with an error matching "unknown schema version 999"

  Scenario: A second concurrent writer is refused
    Given a writer has begun a generation
    When another process writes a generation recording the files "b.rs"
    Then that process fails with an error matching "another writer holds .*write\.lock"
