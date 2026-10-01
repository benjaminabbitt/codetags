# DRAFT for human review (P7.1, for P7.4). Not run: drafts live outside features/.
#
# Questions for review:
#  1. Edge weight is the number of call sites from one side with at least one
#     target on the other, so site 4412 (two targets in ordering) counts once.
#     Intra-module calls are not drawn in the module graph. Agreed?
#  2. Node ids are the name with every character outside [A-Za-z0-9] turned
#     into "_", prefixed "m_" (module), "s_" (symbol) or "g_" (subgraph), so
#     ids stay stable as the code changes and never hit a Mermaid keyword
#     ("end"). Two names that sanitize alike (`a.b`, `a_b`) are not handled
#     yet; a numeric suffix is one option.
#  3. Labels escape "<" and ">" as Mermaid's #lt; and #gt;, because Mermaid
#     strips HTML-like tags from labels. That, the "%%" header line before
#     the diagram type, and "-.->|n|" are unverified: P7.4 must confirm them
#     with the Mermaid CLI (V23) before relying on them.
#  4. A drill-down or call graph draws the symbols in scope inside a subgraph
#     per module, and collapses every endpoint outside the scope into one node
#     per module. A solid edge is a declared resolution; a dotted one exists
#     only through dispatch expansion (cha, vta, ...). Only functions and
#     methods are drawn.
#  5. Scope of @views/callgraph.mmd: the symbols in the result set, plus the
#     symbols defined in the files in it. That is what makes
#     `q/owner=billing` (file tags only, D6) show billing's code. Agreed?
#  6. The .md twin holds the header as an HTML comment, then the diagram
#     without its "%%" header line inside a ```mermaid fence.

Feature: The mermaid plugin draws architecture diagrams
  PLAN.md §2.9 (D8): mermaid is the first declarative plugin, shipped as a
  default. At the root it writes arch/modules.mmd, a module-dependency
  flowchart with edges weighted by call counts, and arch/<module>.mmd, one
  drill-down per module. In every result set it writes
  @views/callgraph.mmd. Each diagram has a .md twin that GitHub and VS Code
  previews render.

  Background:
    Given the view fixture "shop"

  Scenario: arch/ holds the module graph and one drill-down per module, each with a twin
    Then listing "arch" gives exactly "api.md api.mmd billing.md billing.mmd modules.md modules.mmd notify.md notify.mmd ordering.md ordering.mmd"

  Scenario: The module graph weights edges by call sites
    Then reading "arch/modules.mmd" gives:
      """
      %% VIEW of shop @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file
      flowchart LR
        m_api["api"]
        m_billing["billing"]
        m_notify["notify"]
        m_ordering["ordering"]
        m_api -->|1| m_ordering
        m_ordering -->|2| m_billing
        m_ordering -->|1| m_notify
      """

  Scenario: The Markdown twin fences the same diagram
    Then reading "arch/modules.md" gives:
      """
      <!-- VIEW of shop @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file -->
      ```mermaid
      flowchart LR
        m_api["api"]
        m_billing["billing"]
        m_notify["notify"]
        m_ordering["ordering"]
        m_api -->|1| m_ordering
        m_ordering -->|2| m_billing
        m_ordering -->|1| m_notify
      ```
      """

  Scenario: A drill-down shows a module's functions and its neighbours
    Then reading "arch/ordering.mmd" gives:
      """
      %% VIEW of shop @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file
      %% plain module nodes stand for that module's symbols outside this diagram
      %% 1 unresolved call site: see unresolved/ordering.txt
      flowchart LR
        subgraph g_ordering["ordering"]
          s_ordering_OrderRepo_save["ordering.OrderRepo.save"]
          s_ordering_OrderService_place["ordering.OrderService.place"]
          s_ordering_PgOrderRepo_save["ordering.PgOrderRepo.save"]
        end
        m_api["api"]
        m_billing["billing"]
        m_notify["notify"]
        m_api -->|1| s_ordering_OrderService_place
        s_ordering_OrderService_place -->|2| m_billing
        s_ordering_OrderService_place -->|1| m_notify
        s_ordering_OrderService_place -->|1| s_ordering_OrderRepo_save
        s_ordering_OrderService_place -.->|1| s_ordering_PgOrderRepo_save
      """

  Scenario: A drill-down of a leaf module shows who calls it
    Then reading "arch/notify.mmd" gives:
      """
      %% VIEW of shop @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file
      %% plain module nodes stand for that module's symbols outside this diagram
      flowchart LR
        subgraph g_notify["notify"]
          s_notify_Mailer_send["notify.Mailer.send"]
        end
        m_ordering["ordering"]
        m_ordering -->|1| s_notify_Mailer_send
      """

  @query-tree
  Scenario: The call graph of a result set covers just its code
    Then listing "q/owner=billing/@views" gives exactly "callgraph.md callgraph.mmd"
    And reading "q/owner=billing/@views/callgraph.mmd" gives:
      """
      %% VIEW of shop/q/owner=billing @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file
      %% plain module nodes stand for that module's symbols outside this diagram
      flowchart LR
        subgraph g_billing["billing"]
          s_billing_Charge_T__apply["billing.Charge#lt;T#gt;.apply"]
          s_billing_order["billing.order"]
        end
        m_ordering["ordering"]
        m_ordering -->|2| s_billing_Charge_T__apply
        s_billing_Charge_T__apply -->|1| s_billing_order
      """

  @query-tree
  Scenario: The call graph of a quoted query
    Then reading 'q/calls="billing.Charge<T>.apply"/@views/callgraph.mmd' gives:
      """
      %% VIEW of shop/q/calls="billing.Charge<T>.apply" @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file
      %% plain module nodes stand for that module's symbols outside this diagram
      flowchart LR
        subgraph g_ordering["ordering"]
          s_ordering_OrderService_place["ordering.OrderService.place"]
        end
        m_api["api"]
        m_billing["billing"]
        m_notify["notify"]
        m_ordering["ordering"]
        m_api -->|1| s_ordering_OrderService_place
        s_ordering_OrderService_place -->|2| m_billing
        s_ordering_OrderService_place -->|1| m_notify
        s_ordering_OrderService_place -->|1| m_ordering
      """

  @query-tree
  Scenario: The call graph has a Markdown twin
    Then the first line of "q/owner=billing/@views/callgraph.md" is "<!-- VIEW of shop/q/owner=billing @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file -->"
    And "q/owner=billing/@views/callgraph.md" contains the line "```mermaid"

  @mermaid-cli
  Scenario Outline: Every root diagram parses with the Mermaid CLI
    Then "arch/<name>.mmd" parses with the Mermaid CLI

    Examples:
      | name     |
      | modules  |
      | api      |
      | billing  |
      | notify   |
      | ordering |

  @mermaid-cli @query-tree
  Scenario: Result-set diagrams parse with the Mermaid CLI
    Then "q/owner=billing/@views/callgraph.mmd" parses with the Mermaid CLI
    And 'q/calls="billing.Charge<T>.apply"/@views/callgraph.mmd' parses with the Mermaid CLI
