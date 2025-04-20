/**
 * Renders a memory-pack as a woven "tapestry" SVG.
 *
 * The metaphor is a loom: each memory kind is a horizontal *warp* band, and each
 * memory is a *weft* thread laid across it. Thread length encodes confidence,
 * colour encodes kind, and a small knot marks memories that carry links. Detected
 * conflicts are drawn as crossed threads between the two memories involved.
 *
 * The output is a standalone, dependency-free SVG string (no external fonts or
 * images) so it renders anywhere and can be committed to a repo.
 */

import type { MemoryPack, Memory, MemoryKind } from "./pack.js";
import { KINDS } from "./pack.js";

const KIND_COLOR: Record<MemoryKind, string> = {
  episodic: "#e07a5f", // terracotta
  semantic: "#3d8361", // moss
  procedural: "#5f7adb", // indigo
  preference: "#d4a017", // saffron
};

const WIDTH = 900;
const MARGIN = 60;
const BAND_HEIGHT = 90;
const HEADER = 120;
const FOOTER = 70;

function escapeXml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

/** Deterministic pseudo-random in [0,1) from a string seed (FNV-1a based). */
function seededUnit(seed: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < seed.length; i++) {
    h ^= seed.charCodeAt(i);
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return (h % 100000) / 100000;
}

interface ThreadPos {
  id: string;
  x: number;
  y: number;
  color: string;
}

/**
 * Build the SVG string for a pack. The layout is fully deterministic given the
 * pack contents, so re-rendering the same pack yields byte-identical output.
 */
export function renderTapestry(pack: MemoryPack): string {
  const bands = KINDS.filter((k) => (pack.counts_by_kind[k] ?? 0) > 0);
  const height =
    HEADER + FOOTER + Math.max(1, bands.length) * (BAND_HEIGHT + 24);

  const parts: string[] = [];
  parts.push(
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${WIDTH} ${height}" ` +
      `width="${WIDTH}" height="${height}" role="img" ` +
      `aria-label="recallweave memory tapestry with ${pack.live_count} live memories">`,
  );
  parts.push(defs());
  parts.push(
    `<rect x="0" y="0" width="${WIDTH}" height="${height}" fill="url(#linen)"/>`,
  );

  // Title + integrity head.
  parts.push(
    `<text x="${MARGIN}" y="48" font-family="Georgia, 'Times New Roman', serif" ` +
      `font-size="30" fill="#2b2118" font-weight="bold">recallweave tapestry</text>`,
  );
  parts.push(
    `<text x="${MARGIN}" y="76" font-family="monospace" font-size="13" fill="#6b5d4f">` +
      `${pack.live_count} live memories · ${pack.record_count} log records · head ` +
      `${escapeXml(pack.integrity_head.slice(0, 16))}…</text>`,
  );

  // Animated shuttle gliding across the top (the loom's shuttle passing yarn).
  const shuttleTop = 96;
  parts.push(
    `<g transform="translate(0,${shuttleTop})">` +
      `<rect x="-40" y="-7" width="40" height="14" rx="7" fill="#8a5a2b">` +
      `<animate attributeName="x" values="${MARGIN};${WIDTH - MARGIN - 40};${MARGIN}" ` +
      `dur="7s" repeatCount="indefinite"/></rect></g>`,
  );

  const positions: Record<string, ThreadPos> = {};

  bands.forEach((kind, bandIndex) => {
    const bandY = HEADER + bandIndex * (BAND_HEIGHT + 24);
    const color = KIND_COLOR[kind];
    const inBand = pack.memories
      .filter((m) => m.kind === kind)
      .sort((a, b) => a.created_at - b.created_at || a.id.localeCompare(b.id));

    // Warp band background + label.
    parts.push(
      `<rect x="${MARGIN}" y="${bandY}" width="${WIDTH - 2 * MARGIN}" ` +
        `height="${BAND_HEIGHT}" rx="10" fill="${color}" fill-opacity="0.10" ` +
        `stroke="${color}" stroke-opacity="0.5"/>`,
    );
    parts.push(
      `<text x="${MARGIN + 8}" y="${bandY + 20}" font-family="Georgia, serif" ` +
        `font-size="15" fill="${color}" font-weight="bold">${kind} · ${inBand.length}</text>`,
    );

    // Warp threads (faint vertical guide lines).
    for (let gx = MARGIN + 20; gx < WIDTH - MARGIN; gx += 22) {
      parts.push(
        `<line x1="${gx}" y1="${bandY + 28}" x2="${gx}" y2="${bandY + BAND_HEIGHT - 8}" ` +
          `stroke="${color}" stroke-opacity="0.08" stroke-width="1"/>`,
      );
    }

    // Weft threads: one per memory.
    const usable = WIDTH - 2 * MARGIN - 40;
    const step = inBand.length > 0 ? usable / inBand.length : usable;
    inBand.forEach((m, i) => {
      const cx = MARGIN + 30 + step * (i + 0.5);
      const jitter = (seededUnit(m.id) - 0.5) * (BAND_HEIGHT - 50);
      const cy = bandY + BAND_HEIGHT / 2 + 6 + jitter;
      const conf = Math.max(0.05, Math.min(1, m.confidence));
      const len = 10 + conf * 44;
      const faded = m.tombstoned || m.superseded_by !== null;
      const opacity = faded ? 0.25 : 0.95;
      const dash = faded ? ` stroke-dasharray="3 3"` : "";

      // The weft thread, drawn with a gentle sway animation.
      parts.push(
        `<g opacity="${opacity}">` +
          `<line x1="${(cx - len).toFixed(1)}" y1="${cy.toFixed(1)}" ` +
          `x2="${(cx + len).toFixed(1)}" y2="${cy.toFixed(1)}" ` +
          `stroke="${color}" stroke-width="${(1.5 + conf * 3).toFixed(1)}" ` +
          `stroke-linecap="round"${dash}>` +
          `<animate attributeName="y1" values="${cy.toFixed(1)};${(cy - 1.5).toFixed(1)};${cy.toFixed(1)}" ` +
          `dur="${(3 + seededUnit(m.id + "d") * 3).toFixed(2)}s" repeatCount="indefinite"/>` +
          `<animate attributeName="y2" values="${cy.toFixed(1)};${(cy + 1.5).toFixed(1)};${cy.toFixed(1)}" ` +
          `dur="${(3 + seededUnit(m.id + "d") * 3).toFixed(2)}s" repeatCount="indefinite"/>` +
          `</line>` +
          // knot marks memories with outgoing links
          (m.links.length > 0
            ? `<circle cx="${cx.toFixed(1)}" cy="${cy.toFixed(1)}" r="3.2" fill="${color}"/>`
            : "") +
          `<title>${escapeXml(memoryTitle(m))}</title>` +
          `</g>`,
      );
      positions[m.id] = { id: m.id, x: cx, y: cy, color };
    });
  });

  // Conflicts: crossed red threads between the two memories.
  pack.conflicts.forEach((c) => {
    const a = positions[c.a];
    const b = positions[c.b];
    if (!a || !b) return;
    parts.push(
      `<g>` +
        `<line x1="${a.x.toFixed(1)}" y1="${a.y.toFixed(1)}" ` +
        `x2="${b.x.toFixed(1)}" y2="${b.y.toFixed(1)}" stroke="#c1121f" ` +
        `stroke-width="1.6" stroke-dasharray="5 4" opacity="0.85">` +
        `<animate attributeName="stroke-dashoffset" values="0;18" dur="1.2s" ` +
        `repeatCount="indefinite"/></line>` +
        `<title>conflict (${escapeXml(c.kind)}): ${escapeXml(c.explanation)}</title>` +
        `</g>`,
    );
  });

  // Legend / footer.
  const legendY = height - FOOTER + 26;
  let lx = MARGIN;
  for (const kind of KINDS) {
    parts.push(
      `<rect x="${lx}" y="${legendY - 11}" width="14" height="14" rx="3" fill="${KIND_COLOR[kind]}"/>`,
    );
    parts.push(
      `<text x="${lx + 20}" y="${legendY}" font-family="Georgia, serif" font-size="13" ` +
        `fill="#4a3f33">${kind}</text>`,
    );
    lx += 150;
  }
  parts.push(
    `<text x="${MARGIN}" y="${legendY + 24}" font-family="Georgia, serif" font-size="11" ` +
      `fill="#8a7d6d">thread length = confidence · dashed = retired · red = conflict · ` +
      `knot = has links</text>`,
  );

  parts.push(`</svg>`);
  return parts.join("\n");
}

function memoryTitle(m: Memory): string {
  const bits = [
    `[${m.kind}] ${m.content}`,
    `confidence ${m.confidence.toFixed(2)}`,
    `source ${m.provenance.source}`,
  ];
  if (m.tags.length) bits.push(`tags: ${m.tags.join(", ")}`);
  if (m.superseded_by) bits.push(`superseded by ${m.superseded_by}`);
  if (m.tombstoned) bits.push(`tombstoned: ${m.tombstone_reason ?? ""}`);
  return bits.join("\n");
}

function defs(): string {
  return (
    `<defs>` +
    `<pattern id="linen" width="8" height="8" patternUnits="userSpaceOnUse">` +
    `<rect width="8" height="8" fill="#f6f1e7"/>` +
    `<path d="M0 0 L8 0 M0 4 L8 4" stroke="#e8e0d0" stroke-width="0.6"/>` +
    `<path d="M0 0 L0 8 M4 0 L4 8" stroke="#efe8da" stroke-width="0.6"/>` +
    `</pattern>` +
    `</defs>`
  );
}
