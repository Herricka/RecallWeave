//! The memory domain model.
//!
//! A *memory* is a typed, timestamped unit of knowledge an agent wants to keep.
//! Memories are never mutated in place; instead the engine records *events* in an
//! append-only log (see [`crate::store`]). The current state of the store is the
//! result of folding every event in order. This module defines the value types
//! and their conversion to/from the [`Json`] representation used on disk.

use crate::hash::content_fingerprint;
use crate::json::{arr, num, obj, s, Json};
use std::collections::BTreeMap;

/// The four memory kinds the engine understands.
///
/// * `Episodic`   — something that happened at a point in time ("the deploy on
///   Tuesday failed"). Naturally decays; good candidate for TTL.
/// * `Semantic`   — a durable fact ("the prod region is eu-west-1").
/// * `Procedural` — a how-to / recipe ("to rotate the token, run ...").
/// * `Preference` — a stated preference ("prefers concise answers"). These are
///   the most conflict-prone because they change over time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryKind {
    Episodic,
    Semantic,
    Procedural,
    Preference,
}

impl MemoryKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            MemoryKind::Episodic => "episodic",
            MemoryKind::Semantic => "semantic",
            MemoryKind::Procedural => "procedural",
            MemoryKind::Preference => "preference",
        }
    }

    pub fn parse(value: &str) -> Result<MemoryKind, String> {
        match value {
            "episodic" => Ok(MemoryKind::Episodic),
            "semantic" => Ok(MemoryKind::Semantic),
            "procedural" => Ok(MemoryKind::Procedural),
            "preference" => Ok(MemoryKind::Preference),
            other => Err(format!(
                "unknown memory kind '{}' (expected episodic|semantic|procedural|preference)",
                other
            )),
        }
    }
}

/// Where a memory came from. Provenance is first-class: an agent should always
/// be able to answer "why do I believe this?".
#[derive(Debug, Clone, PartialEq)]
pub struct Provenance {
    /// e.g. "user", "tool:web", "inference", "import"
    pub source: String,
    /// Free-form detail: a URL, a conversation id, a tool name, etc.
    pub detail: String,
}

impl Provenance {
    pub fn to_json(&self) -> Json {
        obj(vec![
            ("source", s(&self.source)),
            ("detail", s(&self.detail)),
        ])
    }

    pub fn from_json(v: &Json) -> Result<Provenance, String> {
        Ok(Provenance {
            source: v
                .get("source")
                .and_then(Json::as_str)
                .unwrap_or("unknown")
                .to_string(),
            detail: v
                .get("detail")
                .and_then(Json::as_str)
                .unwrap_or("")
                .to_string(),
        })
    }
}

/// A single memory record in its current, materialized form.
#[derive(Debug, Clone, PartialEq)]
pub struct Memory {
    pub id: String,
    pub kind: MemoryKind,
    pub content: String,
    /// A short lexical fingerprint of the normalized content (dedupe key).
    pub fingerprint: String,
    pub provenance: Provenance,
    /// Subjective confidence in [0.0, 1.0].
    pub confidence: f64,
    /// Sorted, de-duplicated tag list.
    pub tags: Vec<String>,
    /// IDs of related memories (a lightweight knowledge graph).
    pub links: Vec<String>,
    /// Logical creation time (seconds since Unix epoch).
    pub created_at: u64,
    /// Optional time-to-live in seconds. `None` means "keep indefinitely".
    pub ttl_secs: Option<u64>,
    /// If this memory replaced another, the id of the memory it superseded.
    pub supersedes: Option<String>,
    /// If a later memory replaced this one, its id. Set during materialization.
    pub superseded_by: Option<String>,
    /// True once a tombstone event has retired this memory.
    pub tombstoned: bool,
    /// The reason recorded when tombstoned (forgotten, superseded, conflict...).
    pub tombstone_reason: Option<String>,
}

impl Memory {
    /// Has this memory expired relative to `now` (epoch seconds)?
    pub fn is_expired(&self, now: u64) -> bool {
        match self.ttl_secs {
            Some(ttl) => now >= self.created_at.saturating_add(ttl),
            None => false,
        }
    }

    /// Is this memory "live": not tombstoned, not superseded, not expired?
    pub fn is_live(&self, now: u64) -> bool {
        !self.tombstoned && self.superseded_by.is_none() && !self.is_expired(now)
    }

    pub fn to_json(&self) -> Json {
        let mut pairs: Vec<(&str, Json)> = vec![
            ("id", s(&self.id)),
            ("kind", s(self.kind.as_str())),
            ("content", s(&self.content)),
            ("fingerprint", s(&self.fingerprint)),
            ("provenance", self.provenance.to_json()),
            ("confidence", num(self.confidence)),
            ("tags", arr(self.tags.iter().map(|t| s(t)).collect())),
            ("links", arr(self.links.iter().map(|l| s(l)).collect())),
            ("created_at", num(self.created_at as f64)),
            ("tombstoned", Json::Bool(self.tombstoned)),
        ];
        pairs.push((
            "ttl_secs",
            match self.ttl_secs {
                Some(t) => num(t as f64),
                None => Json::Null,
            },
        ));
        pairs.push((
            "supersedes",
            match &self.supersedes {
                Some(x) => s(x),
                None => Json::Null,
            },
        ));
        pairs.push((
            "superseded_by",
            match &self.superseded_by {
                Some(x) => s(x),
                None => Json::Null,
            },
        ));
        pairs.push((
            "tombstone_reason",
            match &self.tombstone_reason {
                Some(x) => s(x),
                None => Json::Null,
            },
        ));
        obj(pairs)
    }

    pub fn from_json(v: &Json) -> Result<Memory, String> {
        let id = req_str(v, "id")?;
        let kind = MemoryKind::parse(&req_str(v, "kind")?)?;
        let content = req_str(v, "content")?;
        let fingerprint = v
            .get("fingerprint")
            .and_then(Json::as_str)
            .map(|x| x.to_string())
            .unwrap_or_else(|| content_fingerprint(&content));
        let provenance = match v.get("provenance") {
            Some(p) => Provenance::from_json(p)?,
            None => Provenance {
                source: "unknown".to_string(),
                detail: String::new(),
            },
        };
        Ok(Memory {
            id,
            kind,
            content,
            fingerprint,
            provenance,
            confidence: v.get("confidence").and_then(Json::as_f64).unwrap_or(1.0),
            tags: str_vec(v, "tags"),
            links: str_vec(v, "links"),
            created_at: v.get("created_at").and_then(Json::as_u64).unwrap_or(0),
            ttl_secs: v.get("ttl_secs").and_then(Json::as_u64),
            supersedes: opt_str(v, "supersedes"),
            superseded_by: opt_str(v, "superseded_by"),
            tombstoned: v.get("tombstoned").and_then(Json::as_bool).unwrap_or(false),
            tombstone_reason: opt_str(v, "tombstone_reason"),
        })
    }
}

/// An event appended to the log. The log is the source of truth; [`Memory`]
/// values are derived by replaying events in order.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A new memory was asserted.
    Assert(Memory),
    /// A memory was retired. Carries the target id and a reason.
    Tombstone { id: String, reason: String },
    /// One memory replaced another. The `new` memory's `supersedes` points at
    /// `old_id`; recording it as a distinct event keeps the intent explicit.
    Supersede { old_id: String, new: Memory },
}

/// A log line: an event plus the integrity chain fields.
#[derive(Debug, Clone, PartialEq)]
pub struct LogRecord {
    /// Monotonic sequence number, starting at 1.
    pub seq: u64,
    /// Wall-clock timestamp the record was written (epoch seconds).
    pub ts: u64,
    pub event: Event,
    /// Digest of the previous record ("0"*64 for the first record).
    pub prev: String,
    /// Chained digest committing to `prev` + this record's payload.
    pub digest: String,
}

impl LogRecord {
    /// The canonical payload bytes over which the digest is computed. It
    /// intentionally excludes `digest` itself but includes everything else, so
    /// any edit to seq/ts/event/prev is detected by verification.
    pub fn payload_json(&self) -> Json {
        let (etype, ebody) = match &self.event {
            Event::Assert(m) => ("assert", m.to_json()),
            Event::Tombstone { id, reason } => {
                ("tombstone", obj(vec![("id", s(id)), ("reason", s(reason))]))
            }
            Event::Supersede { old_id, new } => (
                "supersede",
                obj(vec![("old_id", s(old_id)), ("new", new.to_json())]),
            ),
        };
        obj(vec![
            ("seq", num(self.seq as f64)),
            ("ts", num(self.ts as f64)),
            ("type", s(etype)),
            ("event", ebody),
            ("prev", s(&self.prev)),
        ])
    }

    /// Full on-disk JSON for the record (payload + digest).
    pub fn to_json(&self) -> Json {
        let mut map = match self.payload_json() {
            Json::Obj(m) => m,
            _ => BTreeMap::new(),
        };
        map.insert("digest".to_string(), s(&self.digest));
        Json::Obj(map)
    }

    pub fn from_json(v: &Json) -> Result<LogRecord, String> {
        let seq = v.get("seq").and_then(Json::as_u64).ok_or("missing seq")?;
        let ts = v.get("ts").and_then(Json::as_u64).unwrap_or(0);
        let prev = req_str(v, "prev")?;
        let digest = req_str(v, "digest")?;
        let etype = req_str(v, "type")?;
        let body = v.get("event").ok_or("missing event body")?;
        let event = match etype.as_str() {
            "assert" => Event::Assert(Memory::from_json(body)?),
            "tombstone" => Event::Tombstone {
                id: req_str(body, "id")?,
                reason: body
                    .get("reason")
                    .and_then(Json::as_str)
                    .unwrap_or("")
                    .to_string(),
            },
            "supersede" => Event::Supersede {
                old_id: req_str(body, "old_id")?,
                new: Memory::from_json(body.get("new").ok_or("missing new memory")?)?,
            },
            other => return Err(format!("unknown event type '{}'", other)),
        };
        Ok(LogRecord {
            seq,
            ts,
            event,
            prev,
            digest,
        })
    }
}

// ---- small JSON extraction helpers -----------------------------------------

fn req_str(v: &Json, key: &str) -> Result<String, String> {
    v.get(key)
        .and_then(Json::as_str)
        .map(|x| x.to_string())
        .ok_or_else(|| format!("missing or non-string field '{}'", key))
}

fn opt_str(v: &Json, key: &str) -> Option<String> {
    match v.get(key) {
        Some(Json::Str(x)) => Some(x.clone()),
        _ => None,
    }
}

fn str_vec(v: &Json, key: &str) -> Vec<String> {
    match v.get(key).and_then(Json::as_array) {
        Some(items) => items
            .iter()
            .filter_map(Json::as_str)
            .map(|x| x.to_string())
            .collect(),
        None => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Memory {
        Memory {
            id: "m-1".into(),
            kind: MemoryKind::Preference,
            content: "prefers dark mode".into(),
            fingerprint: content_fingerprint("prefers dark mode"),
            provenance: Provenance {
                source: "user".into(),
                detail: "chat#42".into(),
            },
            confidence: 0.9,
            tags: vec!["ui".into(), "theme".into()],
            links: vec![],
            created_at: 1000,
            ttl_secs: None,
            supersedes: None,
            superseded_by: None,
            tombstoned: false,
            tombstone_reason: None,
        }
    }

    #[test]
    fn memory_round_trips_through_json() {
        let m = sample();
        let j = m.to_json();
        let back = Memory::from_json(&j).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn ttl_expiry_math() {
        let mut m = sample();
        m.ttl_secs = Some(100);
        assert!(!m.is_expired(1050));
        assert!(m.is_expired(1100));
        assert!(m.is_expired(2000));
    }

    #[test]
    fn log_record_round_trips() {
        let rec = LogRecord {
            seq: 1,
            ts: 1234,
            event: Event::Assert(sample()),
            prev: "0".repeat(64),
            digest: "deadbeef".into(),
        };
        let j = rec.to_json();
        let back = LogRecord::from_json(&j).unwrap();
        assert_eq!(rec, back);
    }

    #[test]
    fn kind_parsing_rejects_unknown() {
        assert!(MemoryKind::parse("nonsense").is_err());
        assert_eq!(MemoryKind::parse("semantic").unwrap(), MemoryKind::Semantic);
    }
}
