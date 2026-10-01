Feature: Item ids in the tags file are quoted the tagma way when needed
  The tags file holds one `<id> <tag>...` line per item (PLAN.md §2.3). An id
  is `file:<project-relative POSIX path>` or `sym:<canonical name>`. An id
  holding whitespace or `"` is written as a tagma quoted token, escaping `"`
  as `\"` and `\` as `\\` (D15). tagma keeps a quoted id's quotes (V21), so
  codetags reads ids back itself.

  Scenario Outline: Ids are written and read back
    When the item id for <kind> '<value>' is written
    Then the written id is '<id>'
    And the written id reads back as <kind> '<value>'

    Examples:
      | kind   | value                  | id                         |
      | file   | src/billing/charge.rs  | file:src/billing/charge.rs |
      | file   | docs/My Notes.md       | "file:docs/My Notes.md"    |
      | file   | 100%.txt               | file:100%.txt              |
      | file   | a\\b.txt               | file:a\\b.txt              |
      | file   | a\\ b.txt              | "file:a\\\\ b.txt"         |
      | symbol | billing.Charge<T>.apply | sym:billing.Charge<T>.apply |
      | symbol | billing.Charge.apply+1 | sym:billing.Charge.apply+1 |

  Scenario: A double quote inside an id is escaped
    When the item id for file 'say "hi".txt' is written
    Then the written id is '"file:say \"hi\".txt"'
    And the written id reads back as file 'say "hi".txt'
