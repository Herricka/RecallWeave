

use recallweave::hash;
use recallweave::model::{Memory, MemoryKind};
use recallweave::store::Store;

#[test]
fn verify_rejects_a_tampered_record() {
    let dir = tempdir();
    let mut store = Store::open(&dir).unwrap();
    store.assert_memories(vec![
        Memory::new(MemoryKind::Note, "the user prefers metric units".into(), 1),
        Memory::new(MemoryKind::Note, "deployments happen on tuesdays".into(), 2),
    ]).unwrap();

    assert!(store.verify().is_ok(), "fresh store must verify");

    // Tamper with the first record on disk, exactly as a corrupted export would.
    let path = dir.join("log.jsonl");
    let raw = std::fs::read_to_string(&path).unwrap();
    let tampered = raw.replacen("metric units", "imperial units", 1);
    assert_ne!(raw, tampered, "tamper must actually change the record");
    std::fs::write(&path, tampered).unwrap();

    let reopened = Store::open(&dir).unwrap();
    let err = reopened.verify().expect_err("tampered chain must not verify");
    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("hash") || msg.contains("chain") || msg.contains("integrity"),
        "unhelpful error: {}",
        msg
    );

    std::fs::remove_dir_all(&dir).ok();
}

fn tempdir() -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("recallweave-tamper-{}", std::process::id()));
    std::fs::create_dir_all(&p).unwrap();
    p
}
