# MEMORY.md — the recallweave data model & lifecycle

This document is the reference for how `recallweave` represents an agent's
long‑term memory, how the append‑only log works, and — importantly — an honest
account of what the engine does **not** do.

If you only read one section, read [Honest limitations](#honest-limitations).

---

## 1. What a memory is

A **memory** is a typed, timestamped unit of knowledge. Every memory carries:

| field            | meaning                                                             |
|------------------|---------------------------------------------------------------------|
| `id`             | stable, content‑derived identifier (`mem_<12 hex>`)                 |
| `kind`           | `episodic` \| `semantic` \| `procedural` \| `preference`            |
| `content`        | the human‑readable text                                             |
| `fingerprint`    | 16‑hex lexical fingerprint of the *normalized* content (dedupe key) |
| `provenance`     | `{ source, detail }` — *why do I believe this?*                     |
| `confidence`     | subjective belief in `[0.0, 1.0]`                                   |
| `tags`           | sorted, de‑duplicated labels                                        |
| `links`          | ids of related memories (a light knowledge graph)                   |
| `created_at`     | logical creation time (epoch seconds)                               |
| `ttl_secs`       | optional time‑to‑live; `null` = keep indefinitely                   |
| `supersedes`     | id of a memory this one replaced (if any)                           |
| `superseded_by`  | id of the memory that replaced this one (derived)                   |
| `tombstoned`     | `true` once retired                                                 |
| `tombstone_reason` | why it was retired (`ttl-expired`, `manual`, …)                   |

### The four kinds

- **episodic** — something that happened at a time ("deploy failed at 14:03").
  Naturally decays; the most common candidate for a TTL.
- **semantic** — a durable fact ("prod region is eu‑west‑1").
- **procedural** — a how‑to / recipe ("rotate the token by running …").
- **preference** — a stated preference ("prefers concise answers"). The most
  conflict‑prone, because preferences change over time.

---

## 2. The append‑only log

State lives in a directory (default `.recallweave/`) with a single file,
`log.jsonl`: **one JSON object per line, in append order.** Nothing earlier is
mutated during normal operation. There are three event types:

- `assert` — a new memory is recorded.
- `tombstone` — an existing memory is retired (`{ id, reason }`).
- `supersede` — a new memory replaces an old one (`{ old_id, new }`).

The **current state** is the deterministic fold of every event in order:

```
log.jsonl (events)  ──replay──▶  materialized state (id → Memory)
      ▲                                   │
      │ append                            ▼
 assert/tombstone/supersede    query / verify / export / compact
```

Because state is *derived*, the log is the single source of truth and history is
never lost until you explicitly compact.

### Integrity chain (tamper‑evidence)

Each record stores `prev` (the digest of the previous record) and `digest` (a
256‑bit hash over the record's canonical payload chained with `prev`). The first
record's `prev` is 64 zeros (GENESIS). `recallweave verify` recomputes the whole
chain and reports:

- sequence gaps,
- broken `prev` links,
- any record whose contents no longer match its digest.

This detects accidental corruption and casual tampering. **It is not a
cryptographic signature** — see limitations.

---

## 3. Lifecycle operations

| operation      | command                | effect                                                            |
|----------------|------------------------|-------------------------------------------------------------------|
| assert         | `add`                  | append an `assert` (or `supersede` if `--supersedes` given)       |
| dedupe         | *(automatic on add)*   | identical normalized content + same kind ⇒ reuse existing id      |
| supersede      | `supersede`            | replace an old memory; old becomes `superseded_by`                |
| forget (one)   | `forget <id>`          | append a `tombstone`                                              |
| forget (TTL)   | `gc`                   | tombstone every expired memory                                    |
| conflicts      | `conflicts`            | report (never mutate) detected conflicts                          |
| compaction     | `compact`              | rewrite the log, dropping retired records, rebuilding the chain   |
| verify         | `verify`               | check the integrity chain                                         |
| query / filter | `list`, `query`, `get` | read the materialized state                                       |
| export         | `export`               | write a portable **memory‑pack** for viewers                      |

### Deterministic dedupe

Content is **normalized** before fingerprinting: trim → collapse internal
whitespace → lowercase. So `"Loves  DARK\tmode"` and `"loves dark mode"` collapse
to the same fingerprint. On `add`, if a **live** memory of the **same kind** has
that fingerprint, no new record is written and the existing id is returned. This
is exact/normalized‑lexical dedupe — deterministic and explainable.

### Conflict detection

Two deterministic, explainable detectors run over the live set:

1. **duplicate‑content‑different‑kind** — the same normalized content stored
   under two different kinds (usually a modelling mistake).
2. **preference‑polarity** — two live `preference` memories that share a subject
   token but differ on a recognised antonym from a small built‑in table
   (`dark`/`light`, `concise`/`verbose`, `enable`/`disable`, …). A shared subject
   token is required so `"prefers dark theme"` vs `"prefers light beer"` does
   **not** clash.

Detection never mutates state. You resolve a conflict explicitly by
`supersede`‑ing or `forget`‑ing one side.

### Forgetting & compaction

- **TTL forgetting** (`gc`) tombstones memories whose `created_at + ttl_secs` has
  passed. Expired memories already drop out of `list`/`query`; `gc` makes the
  retirement explicit and durable in the log.
- **Compaction** (`compact`) reads the live set, drops tombstoned / superseded /
  expired records, and re‑weaves the survivors into a fresh log starting from
  GENESIS. Ids and `created_at` are preserved, so the materialized live set is
  identical before and after. Compaction is the *only* operation that rewrites
  history; it is written atomically (temp file + rename).

---

## 4. The memory‑pack export format

`export` produces a single, self‑describing JSON document (`format:
"recallweave-pack"`, `format_version: 1`). It contains only the **live** memories
plus derived statistics, the detected conflicts, and the integrity head. It does
**not** contain the log or the hash chain — viewers never need to understand
chaining. Fields:

```jsonc
{
  "format": "recallweave-pack",
  "format_version": 1,
  "generated_at": 1710000500,
  "integrity_head": "<64 hex>",
  "record_count": 7,
  "live_count": 6,
  "counts_by_kind": { "episodic": 1, "semantic": 1, ... },
  "tag_counts": { "infra": 2, "ui": 2, ... },
  "conflicts": [ { "a", "b", "kind", "explanation" } ],
  "memories": [ /* full Memory objects */ ]
}
```

The TypeScript **tapestry** viewer in `viewer/` consumes exactly this.

---

## 5. Why no dependencies?

The Rust engine uses the **standard library only** — no `serde`, no `sha2`, no
`clap`. This keeps the memory format auditable end‑to‑end (the JSON codec, the
hash, and the CLI parser are all a few hundred readable lines), makes the binary
trivial to build offline, and removes supply‑chain surface for something that
holds an agent's long‑term memory. The trade‑off is that we ship a small JSON
codec and a custom digest instead of reusing mature crates.

---

## <a name="honest-limitations"></a>6. Honest limitations

`recallweave` is a **memory lifecycle engine, not a semantic memory / vector
database.** Being blunt about the boundaries:

- **No semantic matching.** Dedupe, conflict detection, and `query --contains`
  are **purely lexical**. `"prefers dark theme"` and `"wants a dark colour
  scheme"` are, to this engine, unrelated strings. It will not find paraphrases,
  synonyms, or translations. There are no embeddings and no vector index.
- **The conflict detector is a heuristic.** Preference‑polarity relies on a tiny,
  hand‑curated antonym table and a shared‑token check. It will miss real
  conflicts phrased with words outside the table, and it can produce false
  positives when an antonym pair appears coincidentally. It is meant as a *nudge*,
  not a truth oracle.
- **The digest is a checksum, not a cryptographic hash.** `verify` detects
  accidental corruption and casual tampering. It is **not** SHA‑256 and makes no
  preimage/collision‑resistance claims. An attacker who can rewrite the whole
  file can recompute a valid chain. If you need authenticity, sign the exported
  pack with a real signature scheme out of band.
- **Confidence is subjective metadata.** The engine stores and filters on it but
  never *computes* or updates it. There is no Bayesian belief revision.
- **Single‑process, no concurrency control.** The store assumes one writer.
  Concurrent writers to the same directory can interleave and corrupt the log.
  Writes within a process are atomic (temp + rename), but there is no file lock.
- **Time is caller‑supplied logic.** `created_at`/`now` are epoch seconds you can
  override with `--now` for reproducibility. TTL math is simple addition; there
  is no timezone or calendar handling.
- **Full history is retained until compaction.** The log grows with every event.
  `compact` reclaims space but discards the audit trail of retired memories.

These are deliberate choices for a small, auditable, local‑first tool. If your
use case needs semantic recall, run recallweave *alongside* a vector store: use
recallweave for the lifecycle (provenance, TTL, supersession, tombstones,
integrity) and the vector store for similarity search.
