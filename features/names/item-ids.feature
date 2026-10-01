Feature: Item ids in the tags file never need quoting
  The tags file holds one `<id> <tag>...` line per item (PLAN.md §2.3). An id
  is `file:<project-relative POSIX path>` or `sym:<canonical name>`,
  percent-encoded so it never contains whitespace or `"`: tagma keeps a
  quoted id's quotes (V21).

  Scenario Outline: Ids are encoded and decode back
    When the item id for <kind> "<value>" is written
    Then the written id is "<id>"
    And the written id reads back as <kind> "<value>"

    Examples:
      | kind   | value                   | id                          |
      | file   | src/billing/charge.rs   | file:src/billing/charge.rs  |
      | file   | docs/My Notes.md        | file:docs/My%20Notes.md     |
      | file   | 100%.txt                | file:100%25.txt             |
      | symbol | billing.Charge.apply    | sym:billing.Charge.apply    |
      | symbol | billing.Charge.apply+1  | sym:billing.Charge.apply+1  |

  Scenario: A double quote is encoded
    When the item id for file 'say "hi".txt' is written
    Then the written id is "file:say%20%22hi%22.txt"
