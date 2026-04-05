//! Query and filtering over the materialized memory set.
//!
//! Queries are deterministic and purely lexical. A [`Query`] combines optional
//! constraints (kind, tag, substring, minimum confidence, source) that are ANDed
//! together, plus sorting and a limit. There is no ranking model and no semantic
//! matching — substring search means substring search. This is a deliberate,
//! honest design choice documented in `docs/MEMORY.md`.

use crate::hash::normalize_content;
use crate::model::{Memory, MemoryKind};

/// Sort order for query results.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    /// Newest first (descending creation time).
    Newest,
    /// Oldest first (ascending creation time).
    Oldest,
    /// Highest confidence first.
    Confidence,
    /// By id, ascending (fully deterministic tiebreak-free ordering).
    Id,
}

impl Sort {
    pub fn parse(v: &str) -> Result<Sort, String> {
        match v {
            "newest" => Ok(Sort::Newest),
            "oldest" => Ok(Sort::Oldest),
            "confidence" => Ok(Sort::Confidence),
            "id" => Ok(Sort::Id),
            other => Err(format!(
                "unknown sort '{}' (expected newest|oldest|confidence|id)",
                other
            )),
        }
    }
}

/// A conjunctive query over memories.
#[derive(Debug, Clone, Default)]
pub struct Query {
    pub kind: Option<MemoryKind>,
    pub tag: Option<String>,
    /// Case-insensitive substring match against normalized content.
    pub contains: Option<String>,
    pub min_confidence: Option<f64>,
    pub source: Option<String>,
    pub sort: Option<Sort>,
    pub limit: Option<usize>,
}

impl Query {
    /// Apply the query to a set of memories, returning matches in sorted order.
    pub fn run(&self, memories: &[Memory]) -> Vec<Memory> {
        let needle = self.contains.as_ref().map(|c| normalize_content(c));

        let mut out: Vec<Memory> = memories
            .iter()
            .filter(|m| self.kind.map(|k| m.kind == k).unwrap_or(true))
            .filter(|m| {
                self.tag
                    .as_ref()
                    .map(|t| m.tags.iter().any(|x| x == t))
                    .unwrap_or(true)
            })
            .filter(|m| {
                needle
                    .as_ref()
                    .map(|n| normalize_content(&m.content).contains(n.as_str()))
                    .unwrap_or(true)
            })
            .filter(|m| {
                self.min_confidence
                    .map(|c| m.confidence >= c)
                    .unwrap_or(true)
            })
            .filter(|m| {
                self.source
                    .as_ref()
                    .map(|s| &m.provenance.source == s)
                    .unwrap_or(true)
            })
            .cloned()
            .collect();

        match self.sort.unwrap_or(Sort::Newest) {
            Sort::Newest => {
                out.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(a.id.cmp(&b.id)))
            }
            Sort::Oldest => {
                out.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)))
            }
            Sort::Confidence => out.sort_by(|a, b| {
                b.confidence
                    .partial_cmp(&a.confidence)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(a.id.cmp(&b.id))
            }),
            Sort::Id => out.sort_by(|a, b| a.id.cmp(&b.id)),
        }

        if let Some(limit) = self.limit {
            out.truncate(limit);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::content_fingerprint;
    use crate::model::Provenance;

    fn mem(
        id: &str,
        kind: MemoryKind,
        content: &str,
        conf: f64,
        created: u64,
        tags: &[&str],
        source: &str,
    ) -> Memory {
        Memory {
            id: id.into(),
            kind,
            content: content.into(),
            fingerprint: content_fingerprint(content),
            provenance: Provenance {
                source: source.into(),
                detail: String::new(),
            },
            confidence: conf,
            tags: tags.iter().map(|s| s.to_string()).collect(),
            links: vec![],
            created_at: created,
            ttl_secs: None,
            supersedes: None,
            superseded_by: None,
            tombstoned: false,
            tombstone_reason: None,
        }
    }

    fn corpus() -> Vec<Memory> {
        vec![
            mem(
                "a",
                MemoryKind::Semantic,
                "prod region is eu-west-1",
                0.9,
                100,
                &["infra"],
                "user",
            ),
            mem(
                "b",
                MemoryKind::Preference,
                "prefers dark mode",
                0.7,
                200,
                &["ui"],
                "user",
            ),
            mem(
                "c",
                MemoryKind::Episodic,
                "deploy failed on Tuesday",
                0.5,
                300,
                &["infra"],
                "tool",
            ),
        ]
    }

    #[test]
    fn filter_by_kind() {
        let q = Query {
            kind: Some(MemoryKind::Preference),
            ..Default::default()
        };
        let r = q.run(&corpus());
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].id, "b");
    }

    #[test]
    fn filter_by_tag_and_substring() {
        let q = Query {
            tag: Some("infra".into()),
            contains: Some("DEPLOY".into()),
            ..Default::default()
        };
        let r = q.run(&corpus());
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].id, "c");
    }

    #[test]
    fn min_confidence_and_source() {
        let q = Query {
            min_confidence: Some(0.8),
            source: Some("user".into()),
            ..Default::default()
        };
        let r = q.run(&corpus());
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].id, "a");
    }

    #[test]
    fn sort_and_limit() {
        let q = Query {
            sort: Some(Sort::Oldest),
            limit: Some(2),
            ..Default::default()
        };
        let r = q.run(&corpus());
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].id, "a");
        assert_eq!(r[1].id, "b");
    }
}

// feat: add crates/memory handling

// feat: cache parsed crates/memory

// chore: bump deps for cli

// fix: handle empty crates/memory

// docs: add an example for src

// feat: expose tests as a library API

// fix: reject invalid cli instead of failing later

// test: cover src edge cases

// fix: reject invalid crates/memory instead of failing later

// feat: expose benches as a library API

// chore: tidy docs

// docs: add an example for crates/export

// fix: guard against malformed benches

// test: cover crates/export edge cases

// docs: add an example for tests

// docs: add an example for docs

// feat: expose crates/export as a library API

// refactor: split crates/export out of the hot path

// fix: reject invalid crates/export instead of failing later

// test: regression for the src overflow

// feat: cache parsed src

// chore: bump deps for src

// refactor: split src out of the hot path

// refactor: split crates/memory out of the hot path

// test: cover docs edge cases

// feat: cache parsed crates/export

// docs: document cli

// feat: add crates/export handling

// fix: guard against malformed crates/memory

// refactor: rename benches for clarity

// chore: bump deps for crates/memory

// perf: reduce allocations in crates/export

// feat: expose benches as a library API

// test: cover cli edge cases

// docs: document tests

// fix: clamp src to the supported range

// feat: add docs handling

// docs: add an example for crates/export

// docs: document src

// test: regression for the docs overflow

// chore: tidy crates/export

// fix: guard against malformed tests

// feat: expose src as a library API

// fix: clamp cli to the supported range

// docs: add an example for crates/memory

// chore: tidy benches

// docs: document crates/memory

// feat: expose src as a library API

// perf: reduce allocations in crates/memory

// docs: add an example for crates/export

// feat: expose src as a library API

// docs: add an example for crates/export

// fix: reject invalid src instead of failing later

// docs: add an example for docs

// test: regression for the docs overflow

// feat: cache parsed benches

// refactor: rename docs for clarity

// feat: expose src as a library API

// docs: document benches

// fix: handle empty crates/memory

// feat: expose docs as a library API

// fix: guard against malformed benches

// test: regression for the src overflow

// feat: expose tests as a library API

// feat: expose crates/memory as a library API

// refactor: rename docs for clarity

// docs: add an example for docs

// feat: cache parsed cli

// perf: reduce allocations in benches

// refactor: split benches out of the hot path

// test: regression for the crates/memory overflow

// docs: add an example for crates/memory

// feat: expose cli as a library API

// test: regression for the crates/memory overflow

// fix: clamp docs to the supported range

// fix: guard against malformed tests

// feat: expose crates/export as a library API

// refactor: rename crates/export for clarity

// fix: reject invalid docs instead of failing later

// chore: tidy cli

// test: regression for the src overflow

// refactor: split cli out of the hot path

// fix: guard against malformed benches

// test: cover src edge cases

// fix: clamp benches to the supported range

// feat: expose crates/export as a library API

// refactor: split crates/export out of the hot path

// chore: bump deps for crates/export

// refactor: rename tests for clarity

// docs: add an example for crates/memory

// fix: guard against malformed cli

// docs: add an example for cli

// fix: guard against malformed src

// chore: tidy src

// chore: tidy crates/memory
