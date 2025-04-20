//! The `recallweave` command-line interface.
//!
//! A hand-rolled argument parser (no clap; stdlib only) exposes the memory
//! lifecycle as subcommands. Every command operates on a store directory chosen
//! with `--dir` (default `.recallweave`). Output is human-readable text by
//! default, or machine JSON with `--json`, so the CLI is scriptable.
//!
//! Run `recallweave help` for the full command list.

use recallweave::json::Json;
use recallweave::model::{MemoryKind, Provenance};
use recallweave::query::{Query, Sort};
use recallweave::store::{now_epoch, AssertSpec, Store};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {}", e);
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    let mut args: VecDeque<String> = args.into();
    if args.is_empty() {
        print_help();
        return Ok(());
    }
    // Global flags may appear before the subcommand (e.g. `--dir X list`).
    // We scan forward for the first token that is not a flag or a flag value,
    // treating that as the command and leaving all other tokens for the
    // per-command flag parser. `--json`, `--all` are known value-less flags.
    const VALUELESS: &[&str] = &["json", "all", "help", "version"];
    let mut leading: Vec<String> = Vec::new();
    let mut cmd: Option<String> = None;
    while let Some(tok) = args.pop_front() {
        if let Some(name) = tok.strip_prefix("--") {
            // A leading --help/--version acts immediately, regardless of order.
            if name == "help" {
                print_help();
                return Ok(());
            }
            if name == "version" {
                println!("recallweave {}", recallweave::VERSION);
                return Ok(());
            }
            leading.push(tok.clone());
            if !VALUELESS.contains(&name) {
                // consume this flag's value too
                if let Some(v) = args.pop_front() {
                    leading.push(v);
                }
            }
        } else if tok == "-h" {
            print_help();
            return Ok(());
        } else if tok == "-V" {
            println!("recallweave {}", recallweave::VERSION);
            return Ok(());
        } else {
            cmd = Some(tok);
            break;
        }
    }
    let cmd = match cmd {
        Some(c) => c,
        None => {
            // Only global flags were given; nothing to do but show help.
            print_help();
            return Ok(());
        }
    };
    // Re-attach the leading global flags in front of the remaining args so the
    // per-command parser sees them.
    for tok in leading.into_iter().rev() {
        args.push_front(tok);
    }
    match cmd.as_str() {
        "help" | "--help" | "-h" => {
            print_help();
            Ok(())
        }
        "version" | "--version" | "-V" => {
            println!("recallweave {}", recallweave::VERSION);
            Ok(())
        }
        "add" => cmd_add(args),
        "list" => cmd_list(args),
        "query" => cmd_query(args),
        "get" => cmd_get(args),
        "supersede" => cmd_supersede(args),
        "forget" => cmd_forget(args),
        "gc" | "forget-expired" => cmd_forget_expired(args),
        "conflicts" => cmd_conflicts(args),
        "compact" => cmd_compact(args),
        "verify" => cmd_verify(args),
        "export" => cmd_export(args),
        "stats" => cmd_stats(args),
        other => Err(format!(
            "unknown command '{}'. Try 'recallweave help'.",
            other
        )),
    }
}

// ---- flag parsing helpers --------------------------------------------------

/// A tiny option bag: pulls `--key value` and `--flag` out of the arg deque,
/// leaving positional arguments behind.
struct Flags {
    map: std::collections::HashMap<String, String>,
    bools: std::collections::HashSet<String>,
    positionals: Vec<String>,
}

impl Flags {
    fn parse(args: VecDeque<String>, bool_flags: &[&str]) -> Result<Flags, String> {
        let mut map = std::collections::HashMap::new();
        let mut bools = std::collections::HashSet::new();
        let mut positionals = Vec::new();
        let mut it = args.into_iter();
        while let Some(tok) = it.next() {
            if let Some(name) = tok.strip_prefix("--") {
                if bool_flags.contains(&name) {
                    bools.insert(name.to_string());
                } else {
                    let value = it
                        .next()
                        .ok_or_else(|| format!("flag --{} requires a value", name))?;
                    map.insert(name.to_string(), value);
                }
            } else {
                positionals.push(tok);
            }
        }
        Ok(Flags {
            map,
            bools,
            positionals,
        })
    }

    fn get(&self, key: &str) -> Option<&str> {
        self.map.get(key).map(|s| s.as_str())
    }

    fn get_or(&self, key: &str, default: &str) -> String {
        self.map
            .get(key)
            .cloned()
            .unwrap_or_else(|| default.to_string())
    }

    fn has(&self, key: &str) -> bool {
        self.bools.contains(key)
    }

    fn dir(&self) -> PathBuf {
        PathBuf::from(self.get_or("dir", ".recallweave"))
    }
}

fn parse_tags(raw: Option<&str>) -> Vec<String> {
    match raw {
        Some(s) if !s.is_empty() => s
            .split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

// ---- commands --------------------------------------------------------------

fn cmd_add(args: VecDeque<String>) -> Result<(), String> {
    let flags = Flags::parse(args, &["json"])?;
    let kind = MemoryKind::parse(
        flags
            .get("kind")
            .ok_or("add requires --kind <episodic|semantic|procedural|preference>")?,
    )?;
    let content = flags
        .get("content")
        .or_else(|| flags.positionals.first().map(|s| s.as_str()))
        .ok_or("add requires --content <text> (or a positional content argument)")?
        .to_string();
    let confidence: f64 = flags
        .get_or("confidence", "1.0")
        .parse()
        .map_err(|_| "confidence must be a number in [0,1]".to_string())?;
    let ttl = match flags.get("ttl") {
        Some(v) => Some(
            v.parse::<u64>()
                .map_err(|_| "ttl must be a non-negative integer (seconds)".to_string())?,
        ),
        None => None,
    };
    let spec = AssertSpec {
        kind,
        content,
        provenance: Provenance {
            source: flags.get_or("source", "user"),
            detail: flags.get_or("detail", ""),
        },
        confidence,
        tags: parse_tags(flags.get("tags")),
        links: parse_tags(flags.get("links")),
        ttl_secs: ttl,
        supersedes: flags.get("supersedes").map(|s| s.to_string()),
    };

    let mut store = Store::open(&flags.dir())?;
    let now = clock(&flags);
    let (id, deduped) = store.assert(spec, now)?;
    if flags.has("json") {
        let obj = recallweave::json::obj(vec![
            ("id", recallweave::json::s(&id)),
            ("deduped", Json::Bool(deduped)),
        ]);
        println!("{}", obj.to_compact());
    } else if deduped {
        println!("deduped -> existing memory {}", id);
    } else {
        println!("added {}", id);
    }
    Ok(())
}

fn cmd_list(args: VecDeque<String>) -> Result<(), String> {
    let flags = Flags::parse(args, &["json", "all"])?;
    let store = Store::open(&flags.dir())?;
    let now = clock(&flags);
    let mems = if flags.has("all") {
        store.all()
    } else {
        store.live(now)
    };
    print_memories(&mems, flags.has("json"));
    Ok(())
}

fn cmd_query(args: VecDeque<String>) -> Result<(), String> {
    let flags = Flags::parse(args, &["json", "all"])?;
    let store = Store::open(&flags.dir())?;
    let now = clock(&flags);
    let pool = if flags.has("all") {
        store.all()
    } else {
        store.live(now)
    };
    let query = Query {
        kind: match flags.get("kind") {
            Some(k) => Some(MemoryKind::parse(k)?),
            None => None,
        },
        tag: flags.get("tag").map(|s| s.to_string()),
        contains: flags.get("contains").map(|s| s.to_string()),
        min_confidence: match flags.get("min-confidence") {
            Some(v) => Some(
                v.parse()
                    .map_err(|_| "min-confidence must be a number".to_string())?,
            ),
            None => None,
        },
        source: flags.get("source").map(|s| s.to_string()),
        sort: match flags.get("sort") {
            Some(s) => Some(Sort::parse(s)?),
            None => None,
        },
        limit: match flags.get("limit") {
            Some(v) => Some(
                v.parse()
                    .map_err(|_| "limit must be an integer".to_string())?,
            ),
            None => None,
        },
    };
    let results = query.run(&pool);
    print_memories(&results, flags.has("json"));
    Ok(())
}

fn cmd_get(args: VecDeque<String>) -> Result<(), String> {
    let flags = Flags::parse(args, &["json"])?;
    let id = flags
        .positionals
        .first()
        .or_else(|| flags.map.get("id"))
        .ok_or("get requires an id (positional or --id)")?
        .clone();
    let store = Store::open(&flags.dir())?;
    let map = store.materialize();
    let mem = map
        .get(&id)
        .ok_or_else(|| format!("no memory with id '{}'", id))?;
    if flags.has("json") {
        println!("{}", mem.to_json().to_pretty());
    } else {
        print_memory_detail(mem);
    }
    Ok(())
}

fn cmd_supersede(args: VecDeque<String>) -> Result<(), String> {
    let flags = Flags::parse(args, &["json"])?;
    let old_id = flags
        .get("old")
        .ok_or("supersede requires --old <id>")?
        .to_string();
    let content = flags
        .get("content")
        .ok_or("supersede requires --content <text>")?
        .to_string();
    let store_dir = flags.dir();
    let mut store = Store::open(&store_dir)?;
    let now = clock(&flags);
    // Inherit kind from the old memory unless overridden.
    let old_kind = {
        let map = store.materialize();
        let m = map
            .get(&old_id)
            .ok_or_else(|| format!("no memory with id '{}'", old_id))?;
        m.kind
    };
    let kind = match flags.get("kind") {
        Some(k) => MemoryKind::parse(k)?,
        None => old_kind,
    };
    let confidence: f64 = flags
        .get_or("confidence", "1.0")
        .parse()
        .map_err(|_| "confidence must be a number in [0,1]".to_string())?;
    let spec = AssertSpec {
        kind,
        content,
        provenance: Provenance {
            source: flags.get_or("source", "user"),
            detail: flags.get_or("detail", ""),
        },
        confidence,
        tags: parse_tags(flags.get("tags")),
        links: parse_tags(flags.get("links")),
        ttl_secs: None,
        supersedes: Some(old_id.clone()),
    };
    let (new_id, _) = store.assert(spec, now)?;
    if flags.has("json") {
        let obj = recallweave::json::obj(vec![
            ("old", recallweave::json::s(&old_id)),
            ("new", recallweave::json::s(&new_id)),
        ]);
        println!("{}", obj.to_compact());
    } else {
        println!("{} superseded by {}", old_id, new_id);
    }
    Ok(())
}

fn cmd_forget(args: VecDeque<String>) -> Result<(), String> {
    let flags = Flags::parse(args, &["json"])?;
    let id = flags
        .positionals
        .first()
        .or_else(|| flags.map.get("id"))
        .ok_or("forget requires an id (positional or --id)")?
        .clone();
    let reason = flags.get_or("reason", "manual");
    let mut store = Store::open(&flags.dir())?;
    let now = clock(&flags);
    store.tombstone(&id, &reason, now)?;
    if flags.has("json") {
        println!("{{\"forgotten\":\"{}\"}}", id);
    } else {
        println!("forgot {} ({})", id, reason);
    }
    Ok(())
}

fn cmd_forget_expired(args: VecDeque<String>) -> Result<(), String> {
    let flags = Flags::parse(args, &["json"])?;
    let mut store = Store::open(&flags.dir())?;
    let now = clock(&flags);
    let forgotten = store.forget_expired(now)?;
    if flags.has("json") {
        let arr =
            recallweave::json::arr(forgotten.iter().map(|i| recallweave::json::s(i)).collect());
        println!("{}", arr.to_compact());
    } else if forgotten.is_empty() {
        println!("nothing expired");
    } else {
        println!("forgot {} expired memories:", forgotten.len());
        for id in forgotten {
            println!("  {}", id);
        }
    }
    Ok(())
}

fn cmd_conflicts(args: VecDeque<String>) -> Result<(), String> {
    let flags = Flags::parse(args, &["json"])?;
    let store = Store::open(&flags.dir())?;
    let now = clock(&flags);
    let conflicts = store.detect_conflicts(now);
    if flags.has("json") {
        let items: Vec<Json> = conflicts
            .iter()
            .map(|c| {
                recallweave::json::obj(vec![
                    ("a", recallweave::json::s(&c.a)),
                    ("b", recallweave::json::s(&c.b)),
                    ("kind", recallweave::json::s(&c.kind)),
                    ("explanation", recallweave::json::s(&c.explanation)),
                ])
            })
            .collect();
        println!("{}", recallweave::json::arr(items).to_pretty());
    } else if conflicts.is_empty() {
        println!("no conflicts detected");
    } else {
        println!("{} conflict(s):", conflicts.len());
        for c in conflicts {
            println!("  [{}] {} <-> {}: {}", c.kind, c.a, c.b, c.explanation);
        }
    }
    Ok(())
}

fn cmd_compact(args: VecDeque<String>) -> Result<(), String> {
    let flags = Flags::parse(args, &["json"])?;
    let mut store = Store::open(&flags.dir())?;
    let now = clock(&flags);
    let report = store.compact(now)?;
    if flags.has("json") {
        let obj = recallweave::json::obj(vec![
            (
                "before_records",
                recallweave::json::num(report.before_records as f64),
            ),
            (
                "after_records",
                recallweave::json::num(report.after_records as f64),
            ),
            (
                "dropped_tombstoned",
                recallweave::json::num(report.dropped_tombstoned as f64),
            ),
            (
                "dropped_superseded",
                recallweave::json::num(report.dropped_superseded as f64),
            ),
            (
                "dropped_expired",
                recallweave::json::num(report.dropped_expired as f64),
            ),
        ]);
        println!("{}", obj.to_compact());
    } else {
        println!(
            "compacted {} -> {} records (dropped {} tombstoned, {} superseded, {} expired)",
            report.before_records,
            report.after_records,
            report.dropped_tombstoned,
            report.dropped_superseded,
            report.dropped_expired
        );
    }
    Ok(())
}

fn cmd_verify(args: VecDeque<String>) -> Result<(), String> {
    let flags = Flags::parse(args, &["json"])?;
    let store = Store::open(&flags.dir())?;
    let report = store.verify();
    if flags.has("json") {
        let errs: Vec<Json> = report
            .errors
            .iter()
            .map(|e| recallweave::json::s(e))
            .collect();
        let obj = recallweave::json::obj(vec![
            ("records", recallweave::json::num(report.records as f64)),
            ("ok", Json::Bool(report.ok)),
            ("errors", recallweave::json::arr(errs)),
        ]);
        println!("{}", obj.to_pretty());
    } else if report.ok {
        println!("integrity OK: {} records, chain intact", report.records);
    } else {
        println!("integrity FAILED ({} records):", report.records);
        for e in &report.errors {
            println!("  - {}", e);
        }
    }
    if report.ok {
        Ok(())
    } else {
        Err("integrity verification failed".to_string())
    }
}

fn cmd_export(args: VecDeque<String>) -> Result<(), String> {
    let flags = Flags::parse(args, &[])?;
    let store = Store::open(&flags.dir())?;
    let now = clock(&flags);
    let pack = store.export_pack(now);
    let text = pack.to_pretty();
    match flags.get("out") {
        Some(path) => {
            std::fs::write(path, text.as_bytes())
                .map_err(|e| format!("writing pack to {}: {}", path, e))?;
            eprintln!("wrote memory-pack to {}", path);
        }
        None => println!("{}", text),
    }
    Ok(())
}

fn cmd_stats(args: VecDeque<String>) -> Result<(), String> {
    let flags = Flags::parse(args, &["json"])?;
    let store = Store::open(&flags.dir())?;
    let now = clock(&flags);
    let pack = store.export_pack(now);
    if flags.has("json") {
        // Reuse the pack minus the memory bodies for a compact summary.
        let mut summary = std::collections::BTreeMap::new();
        for key in [
            "record_count",
            "live_count",
            "counts_by_kind",
            "tag_counts",
            "integrity_head",
        ] {
            if let Some(v) = pack.get(key) {
                summary.insert(key.to_string(), v.clone());
            }
        }
        println!("{}", Json::Obj(summary).to_pretty());
    } else {
        println!(
            "records:    {}",
            pack.get("record_count").and_then(Json::as_u64).unwrap_or(0)
        );
        println!(
            "live:       {}",
            pack.get("live_count").and_then(Json::as_u64).unwrap_or(0)
        );
        if let Some(Json::Obj(kinds)) = pack.get("counts_by_kind") {
            println!("by kind:");
            for (k, v) in kinds {
                println!("  {:<11} {}", k, v.as_u64().unwrap_or(0));
            }
        }
        println!(
            "head:       {}",
            pack.get("integrity_head")
                .and_then(Json::as_str)
                .unwrap_or("")
        );
    }
    Ok(())
}

// ---- output helpers --------------------------------------------------------

fn print_memories(mems: &[recallweave::model::Memory], as_json: bool) {
    if as_json {
        let items: Vec<Json> = mems.iter().map(|m| m.to_json()).collect();
        println!("{}", recallweave::json::arr(items).to_pretty());
        return;
    }
    if mems.is_empty() {
        println!("(no memories)");
        return;
    }
    for m in mems {
        let status = if m.tombstoned {
            " [tombstoned]"
        } else if m.superseded_by.is_some() {
            " [superseded]"
        } else {
            ""
        };
        println!(
            "{}  {:<10} c={:.2}  {}{}",
            m.id,
            m.kind.as_str(),
            m.confidence,
            m.content,
            status
        );
        if !m.tags.is_empty() {
            println!("    tags: {}", m.tags.join(", "));
        }
    }
}

fn print_memory_detail(m: &recallweave::model::Memory) {
    println!("id:            {}", m.id);
    println!("kind:          {}", m.kind.as_str());
    println!("content:       {}", m.content);
    println!("fingerprint:   {}", m.fingerprint);
    println!("confidence:    {:.2}", m.confidence);
    println!(
        "provenance:    {} ({})",
        m.provenance.source, m.provenance.detail
    );
    println!("created_at:    {}", m.created_at);
    println!(
        "ttl_secs:      {}",
        m.ttl_secs
            .map(|t| t.to_string())
            .unwrap_or_else(|| "-".to_string())
    );
    println!(
        "tags:          {}",
        if m.tags.is_empty() {
            "-".to_string()
        } else {
            m.tags.join(", ")
        }
    );
    println!(
        "links:         {}",
        if m.links.is_empty() {
            "-".to_string()
        } else {
            m.links.join(", ")
        }
    );
    println!(
        "supersedes:    {}",
        m.supersedes.clone().unwrap_or_else(|| "-".to_string())
    );
    println!(
        "superseded_by: {}",
        m.superseded_by.clone().unwrap_or_else(|| "-".to_string())
    );
    println!("tombstoned:    {}", m.tombstoned);
    if let Some(r) = &m.tombstone_reason {
        println!("tombstone:     {}", r);
    }
}

/// Resolve "now": honor `--now <epoch>` for reproducible scripting/tests,
/// otherwise use the system clock.
fn clock(flags: &Flags) -> u64 {
    match flags.get("now") {
        Some(v) => v.parse().unwrap_or_else(|_| now_epoch()),
        None => now_epoch(),
    }
}

fn print_help() {
    println!(
        r#"recallweave {version} — local-first agent memory lifecycle engine

USAGE:
    recallweave <command> [flags]

GLOBAL FLAGS:
    --dir <path>        store directory (default: .recallweave)
    --now <epoch>       override "now" in seconds (for reproducible runs/tests)
    --json              emit machine-readable JSON where supported

COMMANDS:
    add                 assert a new memory
        --kind <episodic|semantic|procedural|preference>   (required)
        --content <text>                                    (required)
        --confidence <0..1>   --ttl <seconds>
        --tags a,b,c          --links id1,id2
        --source <s>          --detail <d>   --supersedes <id>

    supersede           replace an existing memory with new content
        --old <id> --content <text> [--kind ...] [--confidence ...]

    list                list live memories (--all includes retired)
    query               filter memories
        --kind ... --tag ... --contains <text> --min-confidence <0..1>
        --source ... --sort <newest|oldest|confidence|id> --limit <n> [--all]
    get <id>            show one memory in detail

    forget <id>         tombstone a memory   [--reason <text>]
    gc                  forget all TTL-expired memories (alias: forget-expired)
    conflicts           detect conflicts among live memories
    compact             rewrite the log, dropping retired memories
    verify              check the append-only integrity chain
    export              write a portable memory-pack   [--out <file.json>]
    stats               summarize the store

    help                show this help
    version             print version

EXAMPLES:
    recallweave add --kind semantic --content "prod region is eu-west-1" --tags infra
    recallweave query --kind preference --contains theme --sort newest
    recallweave export --out memory-pack.json
    recallweave verify

Honest limitation: matching is lexical, not semantic. See docs/MEMORY.md.
"#,
        version = recallweave::VERSION
    );
}
