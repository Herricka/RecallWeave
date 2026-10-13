# Retention and tombstones (sketch)

A tombstone means "still here, do not return it". It is not the same as a delete,
because something else may still point at it.

- default retention: 30 days
- a tombstone referenced by a live record is never collected
- compaction reports what it dropped and why

## Open questions

- Is 30 days right, or should it be per-pack?
- Do we need a manual pin that survives compaction?
