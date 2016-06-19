//! `recallweave` — a local-first agent memory lifecycle engine.
//!
//! This crate is intentionally dependency-free (Rust standard library only). It
//! models an agent's long-term memory as an append-only, integrity-chained log
//! of typed memories and provides the full lifecycle around them: assertion,
//! deterministic dedupe, supersession, tombstoning, TTL-based forgetting,
//! compaction, conflict detection, integrity verification, query/filter, and a
//! portable export format ("memory-pack").
//!
//! The library is exercised by the `recallweave` binary (see `src/main.rs`) and
//! by the integration tests in `tests/`. See `docs/MEMORY.md` for the data model
//! and an honest account of what this engine does and does not do (notably: it
//! performs no semantic/vector matching).

pub mod hash;
pub mod json;
pub mod model;
pub mod query;
pub mod store;

/// The semantic version of the engine and pack format producer.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

// fix: clamp docs to the supported range

// feat: add cli handling

// fix: reject invalid crates/export instead of failing later

// docs: document crates/memory

// docs: add an example for tests

// feat: expose docs as a library API

// fix: handle empty tests

// fix: handle empty crates/memory

// fix: guard against malformed docs

// test: regression for the tests overflow

// refactor: rename cli for clarity

// docs: add an example for crates/memory

// fix: guard against malformed docs

// chore: bump deps for cli
