# Changelog

All notable changes to recallweave are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] — 2026-08-31

The first release: a complete, self-contained memory lifecycle engine.

### Added — engine (Rust, standard library only)

- **Append-only log** (`log.jsonl`) with three event types: `assert`,
  `tombstone`, `supersede`. State is the deterministic replay of the log.
- **Typed memories**: `episodic`, `semantic`, `procedural`, `preference`, each
  carrying id, provenance, confidence, tags, links, TTL, and supersession fields.
- **Content-derived, stable ids** (`mem_<12 hex>`) and a normalized lexical
  **content fingerprint** for deterministic dedupe.
- **Integrity chain**: each record commits to the previous record's 256-bit
  digest; `verify` recomputes the chain and detects gaps, broken links, and
  altered records.
- **Deterministic dedupe** on assert (same normalized content + same kind).
- **Conflict detection**: `duplicate-content-different-kind` and a heuristic
  `preference-polarity` detector with a shared-subject guard.
- **Forgetting**: per-memory tombstones with reasons, and TTL-based `gc`.
- **Compaction**: rewrites the log to drop retired records while preserving the
  live set (ids and creation times) and rebuilding the integrity chain.
- **Query/filter**: by kind, tag, content substring, minimum confidence, and
  source, with sorting (`newest`/`oldest`/`confidence`/`id`) and limit.
- **Portable memory-pack export** (`format: recallweave-pack`, version 1).
- **Hand-written JSON codec** (deterministic, sorted-key serialization) and a
  custom FNV/SplitMix-based 256-bit digest — no third-party crates.
- **CLI** with `add`, `supersede`, `list`, `query`, `get`, `forget`, `gc`,
  `conflicts`, `compact`, `verify`, `export`, `stats`, `help`, `version`;
  supports `--dir`, `--now` (reproducible clock), and `--json`.

### Added — viewer (TypeScript)

- Validating **pack parser** that narrows raw JSON into typed structures and
  rejects non-packs, malformed JSON, unsupported versions, and bad kinds.
- **Text report** renderer with per-kind bars, top tags, conflicts, and memories.
- **Woven SVG tapestry** renderer (deterministic, XML-escaped) with kind bands,
  confidence-scaled weft threads, retired/faded threads, link knots, and animated
  conflict threads.
- `tapestry` CLI: text report by default, `--svg <file|->` for the tapestry.

### Added — project

- 24 Rust unit tests, 3 integration tests, 5 black-box CLI tests, 8 viewer tests.
- Two hand-authored animated SVGs: `docs/assets/memory-loom.svg` and
  `docs/assets/forgetting-weave.svg`.
- Runnable demo (`examples/demo.sh`, `examples/demo.ps1`) and a committed
  example memory-pack.
- `docs/MEMORY.md` reference, textile/loom-themed `README.md`, `Makefile`,
  `LICENSE` (MIT), and GitHub Actions CI.

### Known limitations

- No semantic/vector matching; all matching is lexical.
- The digest is a strong checksum, not a cryptographic hash.
- Single-writer; no cross-process locking.
- Confidence is stored metadata, never computed.

See `docs/MEMORY.md` for the full, honest limitations discussion.

[0.1.0]: https://example.com/recallweave/releases/tag/v0.1.0


<!-- explain the compaction policy and the verified checkpoint rule -->
