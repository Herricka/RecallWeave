/**
 * Renders a memory-pack as a plain-text report for the terminal.
 *
 * This is the default output of the `tapestry` CLI when no `--svg` is requested.
 * It is intentionally compact and dependency-free (no colour libraries) so it
 * behaves in any terminal and in CI logs.
 */

import type { MemoryPack } from "./pack.js";
import { KINDS } from "./pack.js";

function bar(count: number, max: number, width = 24): string {
  if (max <= 0) return "";
  const filled = Math.round((count / max) * width);
  return "█".repeat(filled) + "·".repeat(Math.max(0, width - filled));
}

export function renderReport(pack: MemoryPack): string {
  const lines: string[] = [];
  lines.push("recallweave memory tapestry");
  lines.push("===========================");
  lines.push(`generated at : ${pack.generated_at}`);
  lines.push(`integrity head: ${pack.integrity_head}`);
  lines.push(`records      : ${pack.record_count}`);
  lines.push(`live memories: ${pack.live_count}`);
  lines.push("");

  lines.push("by kind");
  lines.push("-------");
  const maxKind = Math.max(1, ...KINDS.map((k) => pack.counts_by_kind[k] ?? 0));
  for (const kind of KINDS) {
    const c = pack.counts_by_kind[kind] ?? 0;
    lines.push(`${kind.padEnd(11)} ${String(c).padStart(3)} ${bar(c, maxKind)}`);
  }
  lines.push("");

  const tags = Object.entries(pack.tag_counts).sort(
    (a, b) => b[1] - a[1] || a[0].localeCompare(b[0]),
  );
  if (tags.length) {
    lines.push("top tags");
    lines.push("--------");
    for (const [tag, count] of tags.slice(0, 10)) {
      lines.push(`${tag.padEnd(16)} ${count}`);
    }
    lines.push("");
  }

  if (pack.conflicts.length) {
    lines.push(`conflicts (${pack.conflicts.length})`);
    lines.push("---------");
    for (const c of pack.conflicts) {
      lines.push(`  [${c.kind}] ${c.a} <-> ${c.b}`);
      lines.push(`      ${c.explanation}`);
    }
    lines.push("");
  } else {
    lines.push("conflicts: none");
    lines.push("");
  }

  lines.push("memories");
  lines.push("--------");
  const sorted = [...pack.memories].sort(
    (a, b) => b.created_at - a.created_at || a.id.localeCompare(b.id),
  );
  for (const m of sorted) {
    const flags = [
      m.tombstoned ? "tombstoned" : "",
      m.superseded_by ? "superseded" : "",
      m.ttl_secs !== null ? `ttl=${m.ttl_secs}s` : "",
    ]
      .filter(Boolean)
      .join(" ");
    lines.push(
      `${m.id}  ${m.kind.padEnd(10)} c=${m.confidence.toFixed(2)}  ${m.content}` +
        (flags ? `  (${flags})` : ""),
    );
    if (m.tags.length) lines.push(`    tags: ${m.tags.join(", ")}`);
  }

  return lines.join("\n");
}
