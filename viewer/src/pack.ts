/**
 * Typed model and validating parser for a recallweave memory-pack.
 *
 * The pack format is produced by the Rust engine's `export` command. This module
 * is the single point where the viewer validates that a file really is a pack
 * (checking `format` and `format_version`) and narrows it into strongly typed
 * structures. Everything downstream — the report and the SVG tapestry — consumes
 * these types, never raw JSON.
 */

export type MemoryKind = "episodic" | "semantic" | "procedural" | "preference";

export const KINDS: readonly MemoryKind[] = [
  "episodic",
  "semantic",
  "procedural",
  "preference",
];

export interface Provenance {
  source: string;
  detail: string;
}

export interface Memory {
  id: string;
  kind: MemoryKind;
  content: string;
  fingerprint: string;
  provenance: Provenance;
  confidence: number;
  tags: string[];
  links: string[];
  created_at: number;
  ttl_secs: number | null;
  supersedes: string | null;
  superseded_by: string | null;
  tombstoned: boolean;
  tombstone_reason: string | null;
}

export interface Conflict {
  a: string;
  b: string;
  kind: string;
  explanation: string;
}

export interface MemoryPack {
  format: "recallweave-pack";
  format_version: number;
  generated_at: number;
  integrity_head: string;
  record_count: number;
  live_count: number;
  counts_by_kind: Record<string, number>;
  tag_counts: Record<string, number>;
  conflicts: Conflict[];
  memories: Memory[];
}

/** Thrown when a file is not a valid recallweave pack. */
export class PackError extends Error {}

function isObject(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function asString(v: unknown, path: string): string {
  if (typeof v !== "string") throw new PackError(`${path}: expected string`);
  return v;
}

function asNumber(v: unknown, path: string): number {
  if (typeof v !== "number" || Number.isNaN(v)) {
    throw new PackError(`${path}: expected number`);
  }
  return v;
}

function asBool(v: unknown, path: string): boolean {
  if (typeof v !== "boolean") throw new PackError(`${path}: expected boolean`);
  return v;
}

function asStringArray(v: unknown, path: string): string[] {
  if (!Array.isArray(v)) throw new PackError(`${path}: expected array`);
  return v.map((item, i) => asString(item, `${path}[${i}]`));
}

function parseKind(v: unknown, path: string): MemoryKind {
  const s = asString(v, path);
  if (!KINDS.includes(s as MemoryKind)) {
    throw new PackError(`${path}: unknown kind '${s}'`);
  }
  return s as MemoryKind;
}

function parseMemory(v: unknown, path: string): Memory {
  if (!isObject(v)) throw new PackError(`${path}: expected object`);
  const prov = v.provenance;
  if (!isObject(prov)) throw new PackError(`${path}.provenance: expected object`);
  return {
    id: asString(v.id, `${path}.id`),
    kind: parseKind(v.kind, `${path}.kind`),
    content: asString(v.content, `${path}.content`),
    fingerprint: asString(v.fingerprint, `${path}.fingerprint`),
    provenance: {
      source: asString(prov.source, `${path}.provenance.source`),
      detail: asString(prov.detail, `${path}.provenance.detail`),
    },
    confidence: asNumber(v.confidence, `${path}.confidence`),
    tags: asStringArray(v.tags, `${path}.tags`),
    links: asStringArray(v.links, `${path}.links`),
    created_at: asNumber(v.created_at, `${path}.created_at`),
    ttl_secs: v.ttl_secs === null ? null : asNumber(v.ttl_secs, `${path}.ttl_secs`),
    supersedes: v.supersedes === null ? null : asString(v.supersedes, `${path}.supersedes`),
    superseded_by:
      v.superseded_by === null ? null : asString(v.superseded_by, `${path}.superseded_by`),
    tombstoned: asBool(v.tombstoned, `${path}.tombstoned`),
    tombstone_reason:
      v.tombstone_reason === null
        ? null
        : asString(v.tombstone_reason, `${path}.tombstone_reason`),
  };
}

function parseConflict(v: unknown, path: string): Conflict {
  if (!isObject(v)) throw new PackError(`${path}: expected object`);
  return {
    a: asString(v.a, `${path}.a`),
    b: asString(v.b, `${path}.b`),
    kind: asString(v.kind, `${path}.kind`),
    explanation: asString(v.explanation, `${path}.explanation`),
  };
}

function parseNumberMap(v: unknown, path: string): Record<string, number> {
  if (!isObject(v)) throw new PackError(`${path}: expected object`);
  const out: Record<string, number> = {};
  for (const [k, val] of Object.entries(v)) {
    out[k] = asNumber(val, `${path}.${k}`);
  }
  return out;
}

/** Validate and narrow an arbitrary JSON value into a {@link MemoryPack}. */
export function parsePack(raw: unknown): MemoryPack {
  if (!isObject(raw)) throw new PackError("root: expected object");
  if (raw.format !== "recallweave-pack") {
    throw new PackError(
      `not a recallweave pack (format='${String(raw.format)}')`,
    );
  }
  const version = asNumber(raw.format_version, "format_version");
  if (version !== 1) {
    throw new PackError(`unsupported pack format_version ${version} (expected 1)`);
  }
  const memories = Array.isArray(raw.memories)
    ? raw.memories.map((m, i) => parseMemory(m, `memories[${i}]`))
    : (() => {
        throw new PackError("memories: expected array");
      })();
  const conflicts = Array.isArray(raw.conflicts)
    ? raw.conflicts.map((c, i) => parseConflict(c, `conflicts[${i}]`))
    : [];
  return {
    format: "recallweave-pack",
    format_version: version,
    generated_at: asNumber(raw.generated_at, "generated_at"),
    integrity_head: asString(raw.integrity_head, "integrity_head"),
    record_count: asNumber(raw.record_count, "record_count"),
    live_count: asNumber(raw.live_count, "live_count"),
    counts_by_kind: parseNumberMap(raw.counts_by_kind, "counts_by_kind"),
    tag_counts: parseNumberMap(raw.tag_counts, "tag_counts"),
    conflicts,
    memories,
  };
}

/** Parse pack text (throws {@link PackError} on malformed input). */
export function parsePackText(text: string): MemoryPack {
  let json: unknown;
  try {
    json = JSON.parse(text);
  } catch (e) {
    throw new PackError(`invalid JSON: ${(e as Error).message}`);
  }
  return parsePack(json);
}
