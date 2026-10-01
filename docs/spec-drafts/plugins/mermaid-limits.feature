# DRAFT for human review (P7.1, for P7.4). Not run: drafts live outside features/.
#
# Questions for review:
#  1. The limits are configurable (`[plugins.mermaid] max_edges`,
#     `max_text_size` in .codetags/config.toml) so the rule can be tested on
#     a small fixture. Defaults are Mermaid's own (believed 500 and 50,000,
#     ⚠ V23 pending). Should the limits be configurable for users too, or a
#     test-only knob?
#  2. The O-15 default, as this draft reads it, in order, stopping as soon
#     as the diagram fits:
#       a. collapse modules to depth 2 (`app.web.routes` -> `app.web`),
#          summing edge weights; calls that become intra-module are not
#          drawn, as in any module graph;
#       b. drop the lightest edge, ties broken by the larger (from, to)
#          node id first, until both limits hold. Nodes are never dropped.
#     What if the nodes alone exceed max_text_size: collapse to depth 1,
#     or emit an error file?
#  3. The comment gives counts only, not the elided edges, since a list
#     would grow as edges are dropped and could itself break the text limit.
#  4. "max_text_size" counts the whole file, header and comments included,
#     since Mermaid parses all of it.
#  5. Does the same rule apply to drill-downs and @views/callgraph.mmd? This
#     draft assumes yes, but only specifies arch/modules.mmd.

Feature: Mermaid diagrams aggregate to stay under Mermaid's limits
  PLAN.md §2.9 and O-15: Mermaid refuses to render past its edge and text
  limits, so the plugin aggregates instead: it collapses to module depth 2
  and keeps the heaviest edges under the limit, and says in a comment what
  it elided, rather than emitting a diagram that won't render.

  Background:
    Given a view fixture whose module calls are:
      | from                | to                  | calls |
      | app.web.routes      | core.orders.service | 5     |
      | app.web.auth        | core.orders.service | 2     |
      | app.web.auth        | core.users.store    | 3     |
      | app.cli.main        | core.orders.service | 1     |
      | core.orders.service | core.users.store    | 2     |
      | core.orders.service | infra.db.pool       | 4     |
      | core.users.store    | infra.db.pool       | 6     |
      | core.users.cache    | core.users.store    | 3     |
      | app.web.routes      | infra.log           | 1     |

  Scenario: A diagram within the limits is drawn in full
    Given the project file ".codetags/config.toml" contains:
      """
      [plugins.mermaid]
      max_edges = 9
      """
    Then reading "arch/modules.mmd" gives:
      """
      %% VIEW of shop @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file
      flowchart LR
        m_app_cli_main["app.cli.main"]
        m_app_web_auth["app.web.auth"]
        m_app_web_routes["app.web.routes"]
        m_core_orders_service["core.orders.service"]
        m_core_users_cache["core.users.cache"]
        m_core_users_store["core.users.store"]
        m_infra_db_pool["infra.db.pool"]
        m_infra_log["infra.log"]
        m_app_cli_main -->|1| m_core_orders_service
        m_app_web_auth -->|2| m_core_orders_service
        m_app_web_auth -->|3| m_core_users_store
        m_app_web_routes -->|5| m_core_orders_service
        m_app_web_routes -->|1| m_infra_log
        m_core_orders_service -->|2| m_core_users_store
        m_core_orders_service -->|4| m_infra_db_pool
        m_core_users_cache -->|3| m_core_users_store
        m_core_users_store -->|6| m_infra_db_pool
      """

  Scenario: Collapsing to module depth 2 can be enough
    Given the project file ".codetags/config.toml" contains:
      """
      [plugins.mermaid]
      max_edges = 7
      """
    Then reading "arch/modules.mmd" gives:
      """
      %% VIEW of shop @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file
      %% aggregated to stay under the limits (max_edges 7, max_text_size 50000)
      %% modules collapsed to depth 2: 8 modules drawn as 6
      flowchart LR
        m_app_cli["app.cli"]
        m_app_web["app.web"]
        m_core_orders["core.orders"]
        m_core_users["core.users"]
        m_infra_db["infra.db"]
        m_infra_log["infra.log"]
        m_app_cli -->|1| m_core_orders
        m_app_web -->|7| m_core_orders
        m_app_web -->|3| m_core_users
        m_app_web -->|1| m_infra_log
        m_core_orders -->|2| m_core_users
        m_core_orders -->|4| m_infra_db
        m_core_users -->|6| m_infra_db
      """

  Scenario: Past the edge limit, only the heaviest edges are kept
    Given the project file ".codetags/config.toml" contains:
      """
      [plugins.mermaid]
      max_edges = 4
      """
    Then reading "arch/modules.mmd" gives:
      """
      %% VIEW of shop @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file
      %% aggregated to stay under the limits (max_edges 4, max_text_size 50000)
      %% modules collapsed to depth 2: 8 modules drawn as 6
      %% lightest edges elided: 3 edges, 4 calls
      flowchart LR
        m_app_cli["app.cli"]
        m_app_web["app.web"]
        m_core_orders["core.orders"]
        m_core_users["core.users"]
        m_infra_db["infra.db"]
        m_infra_log["infra.log"]
        m_app_web -->|7| m_core_orders
        m_app_web -->|3| m_core_users
        m_core_orders -->|4| m_infra_db
        m_core_users -->|6| m_infra_db
      """

  Scenario: Past the text limit, edges are dropped until the file fits
    Given the project file ".codetags/config.toml" contains:
      """
      [plugins.mermaid]
      max_text_size = 600
      """
    Then "arch/modules.mmd" is at most 600 bytes
    And reading "arch/modules.mmd" gives:
      """
      %% VIEW of shop @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file
      %% aggregated to stay under the limits (max_edges 500, max_text_size 600)
      %% modules collapsed to depth 2: 8 modules drawn as 6
      %% lightest edges elided: 2 edges, 2 calls
      flowchart LR
        m_app_cli["app.cli"]
        m_app_web["app.web"]
        m_core_orders["core.orders"]
        m_core_users["core.users"]
        m_infra_db["infra.db"]
        m_infra_log["infra.log"]
        m_app_web -->|7| m_core_orders
        m_app_web -->|3| m_core_users
        m_core_orders -->|2| m_core_users
        m_core_orders -->|4| m_infra_db
        m_core_users -->|6| m_infra_db
      """

  @mermaid-cli
  Scenario: An aggregated diagram still parses
    Given the project file ".codetags/config.toml" contains:
      """
      [plugins.mermaid]
      max_edges = 4
      """
    Then "arch/modules.mmd" parses with the Mermaid CLI
