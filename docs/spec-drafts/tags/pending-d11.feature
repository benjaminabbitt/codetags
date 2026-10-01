# PLACEHOLDER (P6.1). D11 (editing by tags, PLAN.md §2.10) and D12 (the
# operation queue and recursion guard, §2.11) are unreviewed drafts, and
# O-19 says their [SPEC] features wait until the human confirms them. This
# file only lists what would be added once they are; it has no scenarios.
#
# Questions for review:
# Q1. Once D12 is confirmed, does CLI `untag` go through the operation queue
#     too (so it is journaled and undoable), and is it subject to the 2 s
#     grace window for buffered deletes? §2.11 says every mutating operation
#     from every frontend queues, "the CLI" included, which would make
#     `untag` in the scenarios elsewhere in this directory asynchronous.
# Q2. Does the journal (`.codetags/local/journal`) record CLI `tag` as well
#     as filesystem operations?

@pending-D11
Feature: Tagging through filesystem operations, and the operation queue (pending D11, D12)
  To be specified after D11 and D12 are reviewed. Planned coverage:

  CLI additions (D11, D12):
  - `codetags untag --all <tag>`: removes a tag from every item; explicit and
    journaled (the replacement for `rmdir` of a query directory).
  - `codetags ops pending`, `ops commit`, `ops discard`, `ops undo`; the
    journal's format and its inverse operations.
  - Whether CLI `tag`/`untag` are queued and journaled, and how the CLI is
    an actor for the decaying counter (one invocation = one actor).

  Live-mount tag operations (§2.10 operation mapping, needs P5):
  - create, `touch`, `cp`, `ln` of `q/<Q>/@files/<path>` adds the definite
    atoms of `<Q>`; creating a path that doesn't exist is EPERM (O-16).
  - `mv` between query directories retags as one queued operation.
  - `rm` untags (buffered); `rmdir` of a query directory is EPERM.
  - A query with `or`, `not`, a quantifier, or a relational, `~` or `!=`
    element accepts no tag operations (EINVAL).
  - Reserved names refused through the mount, with the errno chosen in
    review (the CLI messages in reserved.feature are the model).

  The recursion guard (§2.11):
  - `rm -r` over a 100-file query directory untags nothing until
    `ops commit`; a single `rm` commits after the grace window (P6.4).
  - A tripped actor's further destructive operations fail with EPERM until
    it is quiet; the held batch shows in `ops pending` and `@pending`.
  - Content writes through `@files/` are counted and journaled, never held
    (O-18).

  The lock and file-format scenarios in this directory stay as they are: the
  queue's single worker is then the only process that takes `tags.lock` on
  behalf of the mounts, and the CLI either queues through the daemon or, with
  no daemon, writes directly under the same lock.
