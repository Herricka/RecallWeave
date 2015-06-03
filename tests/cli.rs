//! Black-box tests that invoke the compiled `recallweave` binary.
//!
//! Cargo sets `CARGO_BIN_EXE_recallweave` to the path of the built binary for
//! integration tests, so we can drive the real CLI and assert on its output and
//! exit codes. Each test uses an isolated `--dir` and a fixed `--now` so results
//! are deterministic.

use std::env;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_recallweave")
}

fn fresh_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut dir = env::temp_dir();
    dir.push(format!(
        "recallweave_cli_{}_{}_{}",
        tag,
        std::process::id(),
        nanos
    ));
    dir
}

struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run(dir: &PathBuf, args: &[&str]) -> Out {
    let mut cmd = Command::new(bin());
    cmd.arg("--dir").arg(dir);
    for a in args {
        cmd.arg(a);
    }
    let output = cmd.output().expect("failed to execute recallweave binary");
    Out {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

#[test]
fn help_and_version() {
    let dir = fresh_dir("help");
    let h = run(&dir, &["help"]);
    assert_eq!(h.code, 0);
    assert!(h
        .stdout
        .contains("local-first agent memory lifecycle engine"));

    let v = run(&dir, &["version"]);
    assert_eq!(v.code, 0);
    assert!(v.stdout.contains("recallweave"));
}

#[test]
fn add_list_and_dedupe() {
    let dir = fresh_dir("add");
    let a = run(
        &dir,
        &[
            "add",
            "--kind",
            "semantic",
            "--content",
            "prod region is eu-west-1",
            "--tags",
            "infra",
            "--now",
            "1000",
        ],
    );
    assert_eq!(a.code, 0, "stderr: {}", a.stderr);
    assert!(a.stdout.starts_with("added "));

    // Re-add normalized-identical content: should dedupe.
    let b = run(
        &dir,
        &[
            "add",
            "--kind",
            "semantic",
            "--content",
            "PROD region   is eu-west-1",
            "--now",
            "1010",
            "--json",
        ],
    );
    assert_eq!(b.code, 0);
    assert!(b.stdout.contains("\"deduped\":true"), "got: {}", b.stdout);

    let list = run(&dir, &["list", "--now", "1020"]);
    assert_eq!(list.code, 0);
    assert!(list.stdout.contains("prod region is eu-west-1"));
    // Only one live memory despite two adds.
    assert_eq!(list.stdout.matches("semantic").count(), 1);
}

#[test]
fn verify_and_tamper_detection() {
    let dir = fresh_dir("verify");
    run(
        &dir,
        &[
            "add",
            "--kind",
            "semantic",
            "--content",
            "fact A",
            "--now",
            "1000",
        ],
    );
    run(
        &dir,
        &[
            "add",
            "--kind",
            "semantic",
            "--content",
            "fact B",
            "--now",
            "1001",
        ],
    );

    let ok = run(&dir, &["verify"]);
    assert_eq!(ok.code, 0, "stderr: {}", ok.stderr);
    assert!(ok.stdout.contains("integrity OK"));

    // Tamper with the on-disk log directly.
    let log = dir.join("log.jsonl");
    let content = std::fs::read_to_string(&log).unwrap();
    let tampered = content.replace("fact A", "fact HACKED");
    std::fs::write(&log, tampered).unwrap();

    let bad = run(&dir, &["verify"]);
    assert_ne!(bad.code, 0, "verify must fail on tampered log");
    assert!(bad.stdout.contains("integrity FAILED"));
}

#[test]
fn query_export_and_stats() {
    let dir = fresh_dir("query");
    run(
        &dir,
        &[
            "add",
            "--kind",
            "semantic",
            "--content",
            "region eu-west-1",
            "--tags",
            "infra",
            "--now",
            "1000",
        ],
    );
    run(
        &dir,
        &[
            "add",
            "--kind",
            "preference",
            "--content",
            "prefers dark mode",
            "--tags",
            "ui",
            "--now",
            "1001",
        ],
    );
    run(
        &dir,
        &[
            "add",
            "--kind",
            "episodic",
            "--content",
            "deploy failed",
            "--tags",
            "infra",
            "--now",
            "1002",
        ],
    );

    // Query by tag.
    let q = run(&dir, &["query", "--tag", "infra", "--now", "1003"]);
    assert_eq!(q.code, 0);
    assert!(q.stdout.contains("region eu-west-1"));
    assert!(q.stdout.contains("deploy failed"));
    assert!(!q.stdout.contains("prefers dark mode"));

    // Export to a file.
    let pack_path = dir.join("pack.json");
    let e = run(
        &dir,
        &[
            "export",
            "--out",
            pack_path.to_str().unwrap(),
            "--now",
            "1003",
        ],
    );
    assert_eq!(e.code, 0, "stderr: {}", e.stderr);
    let pack = std::fs::read_to_string(&pack_path).unwrap();
    assert!(pack.contains("\"format\": \"recallweave-pack\""));
    assert!(pack.contains("\"live_count\": 3"));

    // Stats JSON.
    let s = run(&dir, &["stats", "--json", "--now", "1003"]);
    assert_eq!(s.code, 0);
    assert!(s.stdout.contains("\"live_count\": 3"));
}

#[test]
fn supersede_forget_and_compact() {
    let dir = fresh_dir("compact");
    let add = run(
        &dir,
        &[
            "add",
            "--kind",
            "semantic",
            "--content",
            "region us-east-1",
            "--now",
            "1000",
            "--json",
        ],
    );
    // Extract id from JSON output {"deduped":false,"id":"mem_..."}
    let id = extract_id(&add.stdout);

    let sup = run(
        &dir,
        &[
            "supersede",
            "--old",
            &id,
            "--content",
            "region eu-west-1",
            "--now",
            "1100",
        ],
    );
    assert_eq!(sup.code, 0, "stderr: {}", sup.stderr);
    assert!(sup.stdout.contains("superseded by"));

    run(
        &dir,
        &[
            "add",
            "--kind",
            "episodic",
            "--content",
            "temporary note",
            "--now",
            "1101",
            "--json",
        ],
    );
    let list_before = run(&dir, &["list", "--now", "1200"]);
    assert!(list_before.stdout.contains("region eu-west-1"));

    // Forget the temporary episodic note by id.
    let ep_id = {
        let out = run(
            &dir,
            &["query", "--kind", "episodic", "--json", "--now", "1200"],
        );
        extract_id(&out.stdout)
    };
    let f = run(
        &dir,
        &["forget", &ep_id, "--reason", "cleanup", "--now", "1201"],
    );
    assert_eq!(f.code, 0, "stderr: {}", f.stderr);

    // Compact and re-verify.
    let c = run(&dir, &["compact", "--now", "1300"]);
    assert_eq!(c.code, 0, "stderr: {}", c.stderr);
    assert!(c.stdout.contains("compacted"));

    let v = run(&dir, &["verify"]);
    assert_eq!(
        v.code, 0,
        "chain must verify after compaction; stderr: {}",
        v.stderr
    );

    // Only the current region fact remains live.
    let list_after = run(&dir, &["list", "--now", "1300"]);
    assert!(list_after.stdout.contains("region eu-west-1"));
    assert!(!list_after.stdout.contains("temporary note"));
    assert!(!list_after.stdout.contains("us-east-1"));
}

/// Pull the first id value out of a JSON blob, tolerating both compact
/// (`"id":"x"`) and pretty (`"id": "x"`) formatting.
fn extract_id(json: &str) -> String {
    let key = "\"id\"";
    let key_pos = json.find(key).expect("no id in output");
    let after = &json[key_pos + key.len()..];
    // Skip the colon and any whitespace, then the opening quote.
    let quote_rel = after.find('"').expect("no opening quote after id key");
    let rest = &after[quote_rel + 1..];
    let end = rest.find('"').expect("unterminated id");
    rest[..end].to_string()
}
