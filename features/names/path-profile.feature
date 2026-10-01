Feature: The path profile spells one query element as one path component
  Every directory under q/ is one tagma postfix element (PLAN.md §2.7). The
  encoder writes the one spelling that is a legal file name on Linux, macOS
  and Windows; the decoder turns any accepted spelling back into the element.

  Scenario Outline: Elements are encoded only as much as some OS requires
    When the query element "<element>" is encoded as a path component
    Then the path component is "<component>"
    And the path component is a legal file name on linux, macos and windows
    And the path component decodes to "<element>" on linux, macos and windows

    Examples: plain tagma tokens need nothing
      | element            | component          |
      | kind=function      | kind=function      |
      | lang=rust          | lang=rust          |
      | and                | and                |
      | .hidden            | .hidden            |

    Examples: characters Windows reserves
      | element            | component              |
      | prov:source=scip   | prov%3Asource=scip     |
      | line>10            | line%3E10              |
      | line<10            | line%3C10              |
      | name~a.b           | name~a.b               |
      | owner=*            | owner=%2A              |
      | a\|b               | a%7Cb                  |
      | a?b                | a%3Fb                  |

    Examples: percent and slash are always encoded
      | element            | component              |
      | ratio=50%          | ratio=50%25            |
      | ends%20            | ends%2520              |
      | path=src/main.rs   | path=src%2Fmain.rs     |

    Examples: names Windows cannot hold
      | element            | component              |
      | con                | %63on                  |
      | NUL.txt            | %4EUL.txt              |
      | com1               | %63om1                 |
      | lpt9.tar.gz        | %6Cpt9.tar.gz          |
      | console            | console                |
      | ends.              | ends%2E                |
      | .                  | %2E                    |
      | ..                 | .%2E                   |

  Scenario: A quoted value keeps its quotes and spaces
    When the query element 'note="two words"' is encoded as a path component
    Then the path component is "note=%22two words%22"
    And the path component decodes to 'note="two words"' on linux, macos and windows

  Scenario: A trailing space is encoded
    When the query element "word " is encoded as a path component
    Then the path component is "word%20"

  Scenario Outline: Raw tagma syntax decodes on Linux and macOS but not on Windows
    Then the path component "<component>" decodes to "<element>" on linux and macos
    And the path component "<component>" does not decode on windows

    Examples:
      | component          | element            |
      | prov:source=scip   | prov:source=scip   |
      | line>10            | line>10            |
      | con                | con                |

  Scenario Outline: Malformed escapes never decode
    Then the path component "<component>" does not decode on linux, macos and windows

    Examples:
      | component  |
      | 50%        |
      | %4         |
      | %zz        |
      | %FF        |
