//! End-to-end lifecycle tests exercised through the public library API.
//!
//! These tests build a store in a fresh temp directory, drive it through a full
//! lifecycle (assert -> dedupe -> supersede -> forget -> compact), and assert on
//! integrity and the exported pack at each stage. They complement the focused
//! unit tests inside each module.

use recallweave::json::Json;
use recallweave::model::{MemoryKind, Provenance};
use recallweave::query::{Query, Sort};
use recallweave::store::{AssertSpec, Store};
use std::env;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn fresh_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut dir = env::temp_dir();
    dir.push(format!(
        "recallweave_it_{}_{}_{}",
        tag,
        std::process::id(),
        nanos
    ));
    dir
}

fn spec(kind: MemoryKind, content: &str, conf: f64, tags: &[&str]) -> AssertSpec {
    AssertSpec {
        kind,
        content: content.to_string(),
        provenance: Provenance {
            source: "user".into(),
            detail: "integration".into(),
        },
        confidence: conf,
        tags: tags.iter().map(|s| s.to_string()).collect(),
        links: vec![],
        ttl_secs: None,
        supersedes: None,
    }
}

#[test]
fn full_lifecycle_end_to_end() {
    let dir = fresh_dir("full");
    let mut store = Store::open(&dir).unwrap();

    // Assert a spread of memory kinds.
    let (region_id, _) = store
        .assert(
            spec(
                MemoryKind::Semantic,
                "prod region is us-east-1",
                0.9,
                &["infra"],
            ),
            1_000,
        )
        .unwrap();
    store
        .assert(
            spec(
                MemoryKind::Preference,
                "prefers concise answers",
                0.8,
                &["style"],
            ),
            1_010,
        )
        .unwrap();
    store
        .assert(
            spec(
                MemoryKind::Procedural,
                "rotate token via ops runbook step 3",
                0.7,
                &["ops"],
            ),
            1_020,
        )
        .unwrap();

    // Episodic memory with a TTL that will expire.
    let mut ep = spec(
        MemoryKind::Episodic,
        "deploy failed at 14:03",
        0.6,
        &["infra"],
    );
    ep.ttl_secs = Some(100);
    let (ep_id, _) = store.assert(ep, 1_030).unwrap();

    assert_eq!(store.live(1_040).len(), 4);
    assert!(store.verify().ok);

    // Dedupe: re-assert the same semantic fact with different whitespace/case.
    let (dup_id, deduped) = store
        .assert(
            spec(
                MemoryKind::Semantic,
                "PROD region   is us-east-1",
                0.95,
                &[],
            ),
            1_050,
        )
        .unwrap();
    assert!(deduped, "normalized-identical content must dedupe");
    assert_eq!(dup_id, region_id);
    assert_eq!(store.live(1_060).len(), 4, "dedupe adds no new live memory");

    // Supersede the region fact.
    let mut sp = spec(
        MemoryKind::Semantic,
        "prod region is eu-west-1",
        0.99,
        &["infra"],
    );
    sp.supersedes = Some(region_id.clone());
    let (new_region, _) = store.assert(sp, 1_100).unwrap();
    let live = store.live(1_110);
    assert!(live.iter().any(|m| m.id == new_region));
    assert!(!live.iter().any(|m| m.id == region_id));

    // TTL forgetting: episodic memory should expire and be tombstoned.
    let forgotten = store.forget_expired(1_200).unwrap();
    assert!(forgotten.contains(&ep_id));
    assert!(!store.live(1_200).iter().any(|m| m.id == ep_id));

    // Query: only preference memories.
    let prefs = Query {
        kind: Some(MemoryKind::Preference),
        sort: Some(Sort::Id),
        ..Default::default()
    }
    .run(&store.live(1_200));
    assert_eq!(prefs.len(), 1);
    assert_eq!(prefs[0].content, "prefers concise answers");

    // Integrity holds throughout the whole history.
    assert!(store.verify().ok);

    // Export produces a well-formed pack.
    let pack = store.export_pack(1_200);
    assert_eq!(
        pack.get("format").and_then(Json::as_str),
        Some("recallweave-pack")
    );
    let live_count = pack.get("live_count").and_then(Json::as_u64).unwrap();
    assert_eq!(live_count, 3, "region(new) + preference + procedural");

    // Compaction removes retired records but preserves live state and integrity.
    let before = store.live(1_200);
    let report = store.compact(1_200).unwrap();
    assert!(report.after_records < report.before_records);
    let after = store.live(1_200);
    assert_eq!(before.len(), after.len());
    let before_ids: Vec<String> = before.iter().map(|m| m.id.clone()).collect();
    let after_ids: Vec<String> = after.iter().map(|m| m.id.clone()).collect();
    assert_eq!(before_ids, after_ids);
    assert!(store.verify().ok, "chain valid after compaction");
}

#[test]
fn conflicts_surface_and_resolve() {
    let dir = fresh_dir("conflict");
    let mut store = Store::open(&dir).unwrap();

    let (dark, _) = store
        .assert(
            spec(MemoryKind::Preference, "prefers dark theme", 0.7, &["ui"]),
            500,
        )
        .unwrap();
    store
        .assert(
            spec(MemoryKind::Preference, "prefers light theme", 0.9, &["ui"]),
            600,
        )
        .unwrap();

    let conflicts = store.detect_conflicts(700);
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].kind, "preference-polarity");

    // Resolve by superseding the older, lower-confidence preference.
    let mut sp = spec(
        MemoryKind::Preference,
        "prefers light theme confirmed",
        0.95,
        &["ui"],
    );
    sp.supersedes = Some(dark);
    store.assert(sp, 800).unwrap();

    let after = store.detect_conflicts(900);
    assert!(after.is_empty(), "supersession resolves the conflict");
    assert!(store.verify().ok);
}

#[test]
fn reload_from_disk_matches_in_memory() {
    let dir = fresh_dir("reload");
    {
        let mut store = Store::open(&dir).unwrap();
        store
            .assert(spec(MemoryKind::Semantic, "one", 1.0, &[]), 10)
            .unwrap();
        store
            .assert(spec(MemoryKind::Semantic, "two", 1.0, &[]), 20)
            .unwrap();
        store
            .assert(spec(MemoryKind::Semantic, "three", 1.0, &[]), 30)
            .unwrap();
    }
    let reopened = Store::open(&dir).unwrap();
    assert_eq!(reopened.live(100).len(), 3);
    assert!(reopened.verify().ok);
    let pack = reopened.export_pack(100);
    assert_eq!(pack.get("live_count").and_then(Json::as_u64), Some(3));
}
