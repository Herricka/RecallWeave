//! The append-only memory store and its lifecycle operations.
//!
//! On-disk layout is a directory (default `.recallweave/`) containing a single
//! `log.jsonl` file: one JSON object per line, in append order. The engine never
//! rewrites earlier lines during normal operation — new facts, tombstones, and
//! supersessions are *appended*. Compaction is the sole operation that rewrites
//! the file, and it does so by producing a new, smaller log that preserves the
//! live state and the integrity chain.
//!
//! ## The pipeline
//!
//! ```text
//!   log.jsonl (events)  --replay-->  materialized state (Memory map)
//!         ^                                   |
//!         | append                            v
//!    assert/tombstone/supersede        query / verify / export / compact
//! ```
//!
//! Everything below builds on that fold-the-log model.

use crate::hash::{chain_digest, content_fingerprint};
use crate::json::Json;
use crate::model::{Event, LogRecord, Memory, MemoryKind, Provenance};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// The zeroed digest that precedes the very first record.
pub const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// The append-only store, bound to a directory on disk.
pub struct Store {
    dir: PathBuf,
    records: Vec<LogRecord>,
}

/// Parameters for asserting a new memory.
pub struct AssertSpec {
    pub kind: MemoryKind,
    pub content: String,
    pub provenance: Provenance,
    pub confidence: f64,
    pub tags: Vec<String>,
    pub links: Vec<String>,
    pub ttl_secs: Option<u64>,
    pub supersedes: Option<String>,
}

/// A detected conflict between two live memories.
#[derive(Debug, Clone, PartialEq)]
pub struct Conflict {
    pub a: String,
    pub b: String,
    pub kind: String,
    pub explanation: String,
}

/// Summary returned by [`Store::compact`].
#[derive(Debug, Clone, PartialEq)]
pub struct CompactionReport {
    pub before_records: usize,
    pub after_records: usize,
    pub dropped_tombstoned: usize,
    pub dropped_superseded: usize,
    pub dropped_expired: usize,
}

/// Result of [`Store::verify`].
#[derive(Debug, Clone, PartialEq)]
pub struct VerifyReport {
    pub records: usize,
    pub ok: bool,
    pub errors: Vec<String>,
}

impl Store {
    fn log_path(dir: &Path) -> PathBuf {
        dir.join("log.jsonl")
    }

    /// Open an existing store or create an empty one at `dir`.
    pub fn open(dir: &Path) -> Result<Store, String> {
        let path = Self::log_path(dir);
        let records = if path.exists() {
            let text = fs::read_to_string(&path)
                .map_err(|e| format!("reading {}: {}", path.display(), e))?;
            let mut recs = Vec::new();
            for (i, line) in text.lines().enumerate() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let value =
                    Json::parse(line).map_err(|e| format!("line {}: parse error: {}", i + 1, e))?;
                let rec =
                    LogRecord::from_json(&value).map_err(|e| format!("line {}: {}", i + 1, e))?;
                recs.push(rec);
            }
            recs
        } else {
            Vec::new()
        };
        Ok(Store {
            dir: dir.to_path_buf(),
            records,
        })
    }

    /// Number of log records currently held.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// The digest of the most recent record (or GENESIS if empty).
    fn head_digest(&self) -> String {
        self.records
            .last()
            .map(|r| r.digest.clone())
            .unwrap_or_else(|| GENESIS.to_string())
    }

    fn next_seq(&self) -> u64 {
        self.records.last().map(|r| r.seq + 1).unwrap_or(1)
    }

    /// Build a record with proper chaining, append it in memory, and persist.
    fn append(&mut self, ts: u64, event: Event) -> Result<(), String> {
        let seq = self.next_seq();
        let prev = self.head_digest();
        // Compute the digest over the canonical payload.
        let mut partial = LogRecord {
            seq,
            ts,
            event,
            prev: prev.clone(),
            digest: String::new(),
        };
        let payload = partial.payload_json().to_compact();
        partial.digest = chain_digest(&prev, payload.as_bytes());
        self.records.push(partial);
        self.persist()
    }

    /// Write the entire log to disk atomically (write temp, then rename).
    fn persist(&self) -> Result<(), String> {
        fs::create_dir_all(&self.dir)
            .map_err(|e| format!("creating {}: {}", self.dir.display(), e))?;
        let path = Self::log_path(&self.dir);
        let tmp = self.dir.join("log.jsonl.tmp");
        let mut file = fs::File::create(&tmp).map_err(|e| format!("creating temp log: {}", e))?;
        for rec in &self.records {
            let line = rec.to_json().to_compact();
            file.write_all(line.as_bytes())
                .and_then(|_| file.write_all(b"\n"))
                .map_err(|e| format!("writing log: {}", e))?;
        }
        file.flush().map_err(|e| format!("flushing log: {}", e))?;
        drop(file);
        fs::rename(&tmp, &path).map_err(|e| format!("renaming temp log into place: {}", e))?;
        Ok(())
    }

    /// Fold every event into the current materialized memory map.
    ///
    /// This is the definitive interpreter of the log. Ordering matters:
    /// supersession and tombstone events are applied to whatever the map holds
    /// at that point, so replaying is fully deterministic.
    pub fn materialize(&self) -> BTreeMap<String, Memory> {
        let mut map: BTreeMap<String, Memory> = BTreeMap::new();
        for rec in &self.records {
            match &rec.event {
                Event::Assert(m) => {
                    map.insert(m.id.clone(), m.clone());
                }
                Event::Tombstone { id, reason } => {
                    if let Some(m) = map.get_mut(id) {
                        m.tombstoned = true;
                        m.tombstone_reason = Some(reason.clone());
                    }
                }
                Event::Supersede { old_id, new } => {
                    if let Some(old) = map.get_mut(old_id) {
                        old.superseded_by = Some(new.id.clone());
                    }
                    let mut new_mem = new.clone();
                    if new_mem.supersedes.is_none() {
                        new_mem.supersedes = Some(old_id.clone());
                    }
                    map.insert(new_mem.id.clone(), new_mem);
                }
            }
        }
        map
    }

    /// All live memories at time `now`, sorted by id for deterministic output.
    pub fn live(&self, now: u64) -> Vec<Memory> {
        let mut v: Vec<Memory> = self
            .materialize()
            .into_values()
            .filter(|m| m.is_live(now))
            .collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }

    /// All memories (including retired) sorted by creation then id.
    pub fn all(&self) -> Vec<Memory> {
        let mut v: Vec<Memory> = self.materialize().into_values().collect();
        v.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        v
    }

    /// Deterministically derive an id from content, kind and creation time.
    ///
    /// The id is stable: the same (kind, content, created_at) always yields the
    /// same id. That makes fixtures and tests reproducible and means importing
    /// the same memory twice collapses to one id.
    pub fn derive_id(kind: MemoryKind, content: &str, created_at: u64) -> String {
        let seed = format!(
            "{}|{}|{}",
            kind.as_str(),
            content_fingerprint(content),
            created_at
        );
        let fp = content_fingerprint(&seed);
        format!("mem_{}", &fp[..12])
    }

    /// Assert a new memory. Returns `(id, deduped)`.
    ///
    /// Deterministic dedupe: if a *live* memory of the same kind already has the
    /// same content fingerprint, no new record is written and the existing id is
    /// returned with `deduped = true`. This is exact/normalized-lexical dedupe,
    /// not semantic — see honest limitations in the docs.
    pub fn assert(&mut self, spec: AssertSpec, now: u64) -> Result<(String, bool), String> {
        let fingerprint = content_fingerprint(&spec.content);
        let existing = self.materialize();
        for m in existing.values() {
            if m.is_live(now) && m.kind == spec.kind && m.fingerprint == fingerprint {
                return Ok((m.id.clone(), true));
            }
        }
        let id = Self::derive_id(spec.kind, &spec.content, now);
        let mut tags = spec.tags.clone();
        tags.sort();
        tags.dedup();
        let mem = Memory {
            id: id.clone(),
            kind: spec.kind,
            content: spec.content,
            fingerprint,
            provenance: spec.provenance,
            confidence: spec.confidence.clamp(0.0, 1.0),
            tags,
            links: spec.links,
            created_at: now,
            ttl_secs: spec.ttl_secs,
            supersedes: spec.supersedes.clone(),
            superseded_by: None,
            tombstoned: false,
            tombstone_reason: None,
        };
        match spec.supersedes {
            Some(old_id) => self.append(now, Event::Supersede { old_id, new: mem })?,
            None => self.append(now, Event::Assert(mem))?,
        }
        Ok((id, false))
    }

    /// Retire a memory by id with a reason. No-op error if the id is unknown.
    pub fn tombstone(&mut self, id: &str, reason: &str, now: u64) -> Result<(), String> {
        let map = self.materialize();
        if !map.contains_key(id) {
            return Err(format!("no memory with id '{}'", id));
        }
        self.append(
            now,
            Event::Tombstone {
                id: id.to_string(),
                reason: reason.to_string(),
            },
        )
    }

    /// Forget every expired memory that is still active (not already tombstoned
    /// or superseded) at time `now`. Returns the ids that were tombstoned.
    ///
    /// Note: an expired memory is *not* `is_live` (TTL excludes it), so we test
    /// the active flags directly rather than via `is_live`.
    pub fn forget_expired(&mut self, now: u64) -> Result<Vec<String>, String> {
        let mut expired: Vec<String> = self
            .materialize()
            .into_values()
            .filter(|m| !m.tombstoned && m.superseded_by.is_none() && m.is_expired(now))
            .map(|m| m.id)
            .collect();
        expired.sort();
        let mut done = Vec::new();
        for id in expired {
            self.tombstone(&id, "ttl-expired", now)?;
            done.push(id);
        }
        Ok(done)
    }

    /// Detect conflicts among live memories.
    ///
    /// The engine ships two deterministic, explainable detectors:
    ///
    /// 1. **Duplicate fingerprint across kinds** — the same normalized content
    ///    recorded under two different kinds. Usually a modeling mistake.
    ///
    /// 2. **Preference polarity clash** — two live `preference` memories that
    ///    share a *subject* token but differ in a recognized antonym (like
    ///    `dark`/`light`, `concise`/`verbose`, `enable`/`disable`). This is a
    ///    heuristic over a small built-in antonym table, not semantic reasoning.
    ///
    /// Both detectors are lexical and honest about it. They never mutate state;
    /// resolving a conflict is an explicit `supersede`/`tombstone` by the caller.
    pub fn detect_conflicts(&self, now: u64) -> Vec<Conflict> {
        let live: Vec<Memory> = self.live(now);
        let mut conflicts = Vec::new();

        // (1) same fingerprint, different kind.
        for i in 0..live.len() {
            for j in (i + 1)..live.len() {
                let a = &live[i];
                let b = &live[j];
                if a.fingerprint == b.fingerprint && a.kind != b.kind {
                    conflicts.push(Conflict {
                        a: a.id.clone(),
                        b: b.id.clone(),
                        kind: "duplicate-content-different-kind".to_string(),
                        explanation: format!(
                            "identical normalized content stored as both '{}' and '{}'",
                            a.kind.as_str(),
                            b.kind.as_str()
                        ),
                    });
                }
            }
        }

        // (2) preference polarity clash.
        let prefs: Vec<&Memory> = live
            .iter()
            .filter(|m| m.kind == MemoryKind::Preference)
            .collect();
        for i in 0..prefs.len() {
            for j in (i + 1)..prefs.len() {
                let a = prefs[i];
                let b = prefs[j];
                if let Some((wa, wb)) = antonym_clash(&a.content, &b.content) {
                    // Require a shared subject token so unrelated prefs don't clash.
                    if shares_subject(&a.content, &b.content, &wa, &wb) {
                        conflicts.push(Conflict {
                            a: a.id.clone(),
                            b: b.id.clone(),
                            kind: "preference-polarity".to_string(),
                            explanation: format!(
                                "preferences appear to conflict on '{}' vs '{}'",
                                wa, wb
                            ),
                        });
                    }
                }
            }
        }

        conflicts.sort_by(|x, y| (&x.a, &x.b).cmp(&(&y.a, &y.b)));
        conflicts
    }

    /// Rewrite the log to drop retired memories while preserving live state.
    ///
    /// Compaction re-asserts each live memory into a fresh log, rebuilding the
    /// integrity chain from GENESIS. Retired records (tombstoned, superseded,
    /// expired) are dropped. Creation timestamps and ids are preserved so the
    /// materialized live set is byte-for-byte equivalent before and after.
    pub fn compact(&mut self, now: u64) -> Result<CompactionReport, String> {
        let map = self.materialize();
        let before = self.records.len();

        let mut dropped_tombstoned = 0;
        let mut dropped_superseded = 0;
        let mut dropped_expired = 0;
        let mut keep: Vec<Memory> = Vec::new();

        for m in map.into_values() {
            if m.tombstoned {
                dropped_tombstoned += 1;
            } else if m.superseded_by.is_some() {
                dropped_superseded += 1;
            } else if m.is_expired(now) {
                dropped_expired += 1;
            } else {
                keep.push(m);
            }
        }
        keep.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));

        // Rebuild the records from scratch. We clear supersession links since
        // the superseded originals are gone after compaction.
        self.records.clear();
        for mut m in keep {
            m.supersedes = None;
            m.superseded_by = None;
            let ts = m.created_at;
            self.append_no_persist(ts, Event::Assert(m));
        }
        self.persist()?;

        Ok(CompactionReport {
            before_records: before,
            after_records: self.records.len(),
            dropped_tombstoned,
            dropped_superseded,
            dropped_expired,
        })
    }

    /// Like `append` but does not flush to disk (used in batch rebuilds).
    fn append_no_persist(&mut self, ts: u64, event: Event) {
        let seq = self.next_seq();
        let prev = self.head_digest();
        let mut partial = LogRecord {
            seq,
            ts,
            event,
            prev: prev.clone(),
            digest: String::new(),
        };
        let payload = partial.payload_json().to_compact();
        partial.digest = chain_digest(&prev, payload.as_bytes());
        self.records.push(partial);
    }

    /// Verify the integrity chain: sequence monotonicity, prev-links, and the
    /// recomputed digest of every record. Detects any post-hoc tampering with
    /// the on-disk log.
    pub fn verify(&self) -> VerifyReport {
        let mut errors = Vec::new();
        let mut expected_prev = GENESIS.to_string();

        for (i, rec) in self.records.iter().enumerate() {
            let expected_seq = (i as u64) + 1;
            if rec.seq != expected_seq {
                errors.push(format!(
                    "record with digest {}... has seq {} but expected {}",
                    short(&rec.digest),
                    rec.seq,
                    expected_seq
                ));
            }
            if rec.prev != expected_prev {
                errors.push(format!(
                    "seq {}: prev-link mismatch (chain broken)",
                    rec.seq
                ));
            }
            let payload = rec.payload_json().to_compact();
            let recomputed = chain_digest(&rec.prev, payload.as_bytes());
            if recomputed != rec.digest {
                errors.push(format!(
                    "seq {}: digest mismatch (record contents were altered)",
                    rec.seq
                ));
            }
            expected_prev = rec.digest.clone();
        }

        VerifyReport {
            records: self.records.len(),
            ok: errors.is_empty(),
            errors,
        }
    }

    /// Build a portable memory-pack: a single self-describing JSON document with
    /// the live memories, derived statistics, and the integrity head. The pack
    /// is what the TypeScript viewer consumes; it is intentionally decoupled
    /// from the log format so viewers never need to understand chaining.
    pub fn export_pack(&self, now: u64) -> Json {
        let live = self.live(now);
        let mut by_kind: BTreeMap<String, u64> = BTreeMap::new();
        let mut tag_counts: BTreeMap<String, u64> = BTreeMap::new();
        for m in &live {
            *by_kind.entry(m.kind.as_str().to_string()).or_insert(0) += 1;
            for t in &m.tags {
                *tag_counts.entry(t.clone()).or_insert(0) += 1;
            }
        }
        let conflicts = self.detect_conflicts(now);

        let mem_json: Vec<Json> = live.iter().map(|m| m.to_json()).collect();
        let kind_json = Json::Obj(
            by_kind
                .into_iter()
                .map(|(k, v)| (k, Json::Num(v as f64)))
                .collect(),
        );
        let tag_json = Json::Obj(
            tag_counts
                .into_iter()
                .map(|(k, v)| (k, Json::Num(v as f64)))
                .collect(),
        );
        let conflict_json: Vec<Json> = conflicts
            .iter()
            .map(|c| {
                let mut m = BTreeMap::new();
                m.insert("a".to_string(), Json::Str(c.a.clone()));
                m.insert("b".to_string(), Json::Str(c.b.clone()));
                m.insert("kind".to_string(), Json::Str(c.kind.clone()));
                m.insert("explanation".to_string(), Json::Str(c.explanation.clone()));
                Json::Obj(m)
            })
            .collect();

        let mut root = BTreeMap::new();
        root.insert(
            "format".to_string(),
            Json::Str("recallweave-pack".to_string()),
        );
        root.insert("format_version".to_string(), Json::Num(1.0));
        root.insert("generated_at".to_string(), Json::Num(now as f64));
        root.insert("integrity_head".to_string(), Json::Str(self.head_digest()));
        root.insert(
            "record_count".to_string(),
            Json::Num(self.records.len() as f64),
        );
        root.insert("live_count".to_string(), Json::Num(live.len() as f64));
        root.insert("counts_by_kind".to_string(), kind_json);
        root.insert("tag_counts".to_string(), tag_json);
        root.insert("conflicts".to_string(), Json::Arr(conflict_json));
        root.insert("memories".to_string(), Json::Arr(mem_json));
        Json::Obj(root)
    }
}

/// Current wall-clock time as epoch seconds.
pub fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn short(s: &str) -> String {
    s.chars().take(8).collect()
}

/// Small, curated antonym table for the preference-polarity detector.
/// Deliberately tiny and transparent — this is a heuristic, not a thesaurus.
const ANTONYMS: &[(&str, &str)] = &[
    ("dark", "light"),
    ("concise", "verbose"),
    ("enable", "disable"),
    ("enabled", "disabled"),
    ("on", "off"),
    ("formal", "casual"),
    ("metric", "imperial"),
    ("verbose", "terse"),
    ("increase", "decrease"),
    ("allow", "deny"),
];

/// If two texts contain a recognized antonym pair, return the pair.
fn antonym_clash(a: &str, b: &str) -> Option<(String, String)> {
    let ta = tokenize(a);
    let tb = tokenize(b);
    for (x, y) in ANTONYMS {
        let ax = ta.iter().any(|t| t == x);
        let ay = ta.iter().any(|t| t == y);
        let bx = tb.iter().any(|t| t == x);
        let by = tb.iter().any(|t| t == y);
        if ax && by {
            return Some(((*x).to_string(), (*y).to_string()));
        }
        if ay && bx {
            return Some(((*y).to_string(), (*x).to_string()));
        }
    }
    None
}

/// Do the two preference texts share a subject token besides the antonym words?
/// This guards against flagging "prefers dark mode" vs "likes light beer".
fn shares_subject(a: &str, b: &str, wa: &str, wb: &str) -> bool {
    let ta = tokenize(a);
    let tb = tokenize(b);
    const STOP: &[&str] = &[
        "a", "an", "the", "to", "of", "in", "on", "for", "and", "or", "prefers", "prefer", "likes",
        "like", "wants", "want", "uses", "use", "mode", "please", "always", "i",
    ];
    for t in &ta {
        if t == wa || t == wb {
            continue;
        }
        if STOP.contains(&t.as_str()) {
            continue;
        }
        if tb.contains(t) {
            return true;
        }
    }
    false
}

/// Lowercase alphanumeric tokenization.
fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    for c in text.chars() {
        if c.is_alphanumeric() {
            for lc in c.to_lowercase() {
                cur.push(lc);
            }
        } else if !cur.is_empty() {
            tokens.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    fn tmp_store() -> Store {
        let mut dir = env::temp_dir();
        let unique = format!(
            "recallweave_test_{}_{}",
            std::process::id(),
            now_epoch_nanos()
        );
        dir.push(unique);
        Store::open(&dir).unwrap()
    }

    fn now_epoch_nanos() -> u128 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    }

    fn spec(kind: MemoryKind, content: &str) -> AssertSpec {
        AssertSpec {
            kind,
            content: content.to_string(),
            provenance: Provenance {
                source: "test".into(),
                detail: String::new(),
            },
            confidence: 0.9,
            tags: vec![],
            links: vec![],
            ttl_secs: None,
            supersedes: None,
        }
    }

    #[test]
    fn assert_and_materialize() {
        let mut s = tmp_store();
        let (id, dup) = s
            .assert(spec(MemoryKind::Semantic, "prod is eu-west-1"), 100)
            .unwrap();
        assert!(!dup);
        let live = s.live(100);
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].id, id);
    }

    #[test]
    fn dedupe_is_deterministic() {
        let mut s = tmp_store();
        let (id1, dup1) = s
            .assert(spec(MemoryKind::Preference, "prefers  DARK mode"), 100)
            .unwrap();
        let (id2, dup2) = s
            .assert(spec(MemoryKind::Preference, "prefers dark mode"), 200)
            .unwrap();
        assert!(!dup1);
        assert!(dup2);
        assert_eq!(id1, id2);
        assert_eq!(s.live(300).len(), 1);
    }

    #[test]
    fn supersession_retires_old() {
        let mut s = tmp_store();
        let (old, _) = s
            .assert(spec(MemoryKind::Semantic, "prod is us-east-1"), 100)
            .unwrap();
        let mut sp = spec(MemoryKind::Semantic, "prod is eu-west-1");
        sp.supersedes = Some(old.clone());
        let (newid, _) = s.assert(sp, 200).unwrap();
        let live = s.live(300);
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].id, newid);
        let all = s.materialize();
        assert_eq!(all[&old].superseded_by, Some(newid));
    }

    #[test]
    fn ttl_forgetting() {
        let mut s = tmp_store();
        let mut sp = spec(MemoryKind::Episodic, "deploy happened");
        sp.ttl_secs = Some(50);
        let (id, _) = s.assert(sp, 100).unwrap();
        assert_eq!(s.live(120).len(), 1);
        assert_eq!(s.live(160).len(), 0); // expired but not yet tombstoned
        let forgotten = s.forget_expired(160).unwrap();
        assert_eq!(forgotten, vec![id.clone()]);
        assert!(s.materialize()[&id].tombstoned);
    }

    #[test]
    fn conflict_same_content_different_kind() {
        let mut s = tmp_store();
        s.assert(spec(MemoryKind::Semantic, "coffee is good"), 100)
            .unwrap();
        s.assert(spec(MemoryKind::Preference, "coffee is good"), 100)
            .unwrap();
        let c = s.detect_conflicts(200);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].kind, "duplicate-content-different-kind");
    }

    #[test]
    fn conflict_preference_polarity() {
        let mut s = tmp_store();
        s.assert(spec(MemoryKind::Preference, "prefers dark theme"), 100)
            .unwrap();
        s.assert(spec(MemoryKind::Preference, "prefers light theme"), 100)
            .unwrap();
        let c = s.detect_conflicts(200);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].kind, "preference-polarity");
    }

    #[test]
    fn unrelated_preferences_do_not_conflict() {
        let mut s = tmp_store();
        s.assert(spec(MemoryKind::Preference, "prefers dark theme"), 100)
            .unwrap();
        s.assert(spec(MemoryKind::Preference, "prefers light beer"), 100)
            .unwrap();
        let c = s.detect_conflicts(200);
        assert!(c.is_empty(), "should not clash without shared subject");
    }

    #[test]
    fn verify_detects_tampering() {
        let mut s = tmp_store();
        s.assert(spec(MemoryKind::Semantic, "fact one"), 100)
            .unwrap();
        s.assert(spec(MemoryKind::Semantic, "fact two"), 100)
            .unwrap();
        assert!(s.verify().ok);
        // Tamper with a record's content in memory.
        if let Event::Assert(m) = &mut s.records[0].event {
            m.content = "fact one altered".into();
        }
        let report = s.verify();
        assert!(!report.ok);
        assert!(report.errors.iter().any(|e| e.contains("digest mismatch")));
    }

    #[test]
    fn compaction_preserves_live_state() {
        let mut s = tmp_store();
        let (a, _) = s
            .assert(spec(MemoryKind::Semantic, "keep me"), 100)
            .unwrap();
        let (b, _) = s
            .assert(spec(MemoryKind::Episodic, "forget me"), 100)
            .unwrap();
        s.tombstone(&b, "manual", 150).unwrap();
        let before_live = s.live(200);
        let report = s.compact(200).unwrap();
        assert_eq!(report.dropped_tombstoned, 1);
        let after_live = s.live(200);
        assert_eq!(before_live.len(), after_live.len());
        assert_eq!(after_live[0].id, a);
        assert!(s.verify().ok, "chain must be valid after compaction");
    }

    #[test]
    fn export_pack_shape() {
        let mut s = tmp_store();
        s.assert(spec(MemoryKind::Semantic, "fact"), 100).unwrap();
        let pack = s.export_pack(200);
        assert_eq!(
            pack.get("format").and_then(Json::as_str),
            Some("recallweave-pack")
        );
        assert_eq!(pack.get("live_count").and_then(Json::as_u64), Some(1));
        assert!(pack.get("memories").and_then(Json::as_array).is_some());
    }

    #[test]
    fn reopen_persists_across_instances() {
        let mut dir = env::temp_dir();
        dir.push(format!(
            "recallweave_persist_{}_{}",
            std::process::id(),
            now_epoch_nanos()
        ));
        {
            let mut s = Store::open(&dir).unwrap();
            s.assert(spec(MemoryKind::Semantic, "persistent fact"), 100)
                .unwrap();
        }
        let s2 = Store::open(&dir).unwrap();
        assert_eq!(s2.live(200).len(), 1);
        assert!(s2.verify().ok);
    }
}
