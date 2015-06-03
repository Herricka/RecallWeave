/**
 * Tests for the tapestry viewer, using Node's built-in assert (no test runner
 * dependency). Run with `npm test`, which compiles then executes this file.
 * The script exits non-zero if any assertion fails.
 */

import assert from "node:assert/strict";
import { parsePackText, PackError, type MemoryPack } from "./pack.js";
import { renderReport } from "./report.js";
import { renderTapestry } from "./svg.js";

let passed = 0;
function test(name: string, fn: () => void): void {
  try {
    fn();
    passed++;
    process.stdout.write(`ok   - ${name}\n`);
  } catch (e) {
    process.stdout.write(`FAIL - ${name}\n  ${(e as Error).message}\n`);
    process.exitCode = 1;
  }
}

function samplePackText(): string {
  const pack: MemoryPack = {
    format: "recallweave-pack",
    format_version: 1,
    generated_at: 1710000500,
    integrity_head: "abc123def456abc123def456abc123def456abc123def456abc123def4560000",
    record_count: 4,
    live_count: 3,
    counts_by_kind: { semantic: 1, preference: 2, episodic: 0, procedural: 0 },
    tag_counts: { ui: 2, infra: 1 },
    conflicts: [
      {
        a: "mem_a",
        b: "mem_b",
        kind: "preference-polarity",
        explanation: "preferences appear to conflict on 'dark' vs 'light'",
      },
    ],
    memories: [
      {
        id: "mem_region",
        kind: "semantic",
        content: "prod region is eu-west-1",
        fingerprint: "0000000000000000",
        provenance: { source: "user", detail: "" },
        confidence: 0.99,
        tags: ["infra"],
        links: [],
        created_at: 1710000100,
        ttl_secs: null,
        supersedes: null,
        superseded_by: null,
        tombstoned: false,
        tombstone_reason: null,
      },
      {
        id: "mem_a",
        kind: "preference",
        content: "prefers dark theme",
        fingerprint: "1111111111111111",
        provenance: { source: "user", detail: "" },
        confidence: 0.7,
        tags: ["ui"],
        links: ["mem_region"],
        created_at: 1710000200,
        ttl_secs: null,
        supersedes: null,
        superseded_by: null,
        tombstoned: false,
        tombstone_reason: null,
      },
      {
        id: "mem_b",
        kind: "preference",
        content: "prefers light theme",
        fingerprint: "2222222222222222",
        provenance: { source: "user", detail: "" },
        confidence: 0.6,
        tags: ["ui"],
        links: [],
        created_at: 1710000200,
        ttl_secs: null,
        supersedes: null,
        superseded_by: null,
        tombstoned: false,
        tombstone_reason: null,
      },
    ],
  };
  return JSON.stringify(pack);
}

test("parsePack accepts a valid pack", () => {
  const pack = parsePackText(samplePackText());
  assert.equal(pack.live_count, 3);
  assert.equal(pack.memories.length, 3);
  assert.equal(pack.conflicts.length, 1);
});

test("parsePack rejects non-pack JSON", () => {
  assert.throws(() => parsePackText('{"format":"nope"}'), PackError);
});

test("parsePack rejects malformed JSON", () => {
  assert.throws(() => parsePackText("{not json"), PackError);
});

test("parsePack rejects unsupported version", () => {
  assert.throws(
    () => parsePackText('{"format":"recallweave-pack","format_version":99}'),
    PackError,
  );
});

test("parsePack rejects bad memory kind", () => {
  const bad = JSON.parse(samplePackText());
  bad.memories[0].kind = "telepathic";
  assert.throws(() => parsePackText(JSON.stringify(bad)), PackError);
});

test("report includes counts, conflicts and memories", () => {
  const report = renderReport(parsePackText(samplePackText()));
  assert.ok(report.includes("live memories: 3"));
  assert.ok(report.includes("preference-polarity"));
  assert.ok(report.includes("prefers dark theme"));
  assert.ok(report.includes("by kind"));
});

test("svg is well-formed and deterministic", () => {
  const pack = parsePackText(samplePackText());
  const svg1 = renderTapestry(pack);
  const svg2 = renderTapestry(pack);
  assert.equal(svg1, svg2, "rendering must be deterministic");
  assert.ok(svg1.startsWith("<svg"));
  assert.ok(svg1.trimEnd().endsWith("</svg>"));
  // conflict thread present
  assert.ok(svg1.includes("#c1121f"));
  // both preference memories referenced via titles
  assert.ok(svg1.includes("prefers dark theme"));
  assert.ok(svg1.includes("prefers light theme"));
});

test("svg escapes special characters in content", () => {
  const bad = JSON.parse(samplePackText());
  bad.memories[0].content = 'danger <script> & "quotes"';
  const svg = renderTapestry(parsePackText(JSON.stringify(bad)));
  assert.ok(!svg.includes("<script>"), "raw markup must be escaped");
  assert.ok(svg.includes("&lt;script&gt;"));
});

process.stdout.write(`\n${passed} passed\n`);
