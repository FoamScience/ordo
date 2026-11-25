// A definition's readability measurements against its own old version, said
// the way the engine's summary says them: `nesting 2 → 5`.

import type { Readability } from "./types.ts";

const METRICS: [keyof Omit<Readability, "name">, string, string][] = [
  ["nesting", "nesting", ""],
  ["cognitive", "cognitive", ""],
  ["branches", "branches", ""],
  ["params", "params", ""],
  ["short_names", "short names", "%"],
  ["lines", "lines", ""],
];

/** The measurements that moved either way, in the order the engine ranks them. */
export function moved(d: Readability): string[] {
  return METRICS.filter(([k]) => d[k][0] !== d[k][1]).map(([k, name, unit]) => `${name} ${d[k][0]}${unit} → ${d[k][1]}${unit}`);
}

/** Something other than its length grew: harder to read, not just longer. */
export function regressed(d: Readability): boolean {
  return METRICS.some(([k]) => k !== "lines" && d[k][1] > d[k][0]);
}

const SHORT: Record<string, string> = { nesting: "nest", cognitive: "cx", branches: "br", params: "par", short_names: "short", lines: "lines" };

/** The measurements that moved, each short, `cx 4→7`, with `cognitive 4 to 7` for the words. */
export function deltas(d: Readability): { text: string; words: string }[] {
  return METRICS.filter(([k]) => d[k][0] !== d[k][1]).map(([k, name, unit]) => ({
    text: `${SHORT[k]} ${d[k][0]}→${d[k][1]}${unit}`,
    words: `${name} ${d[k][0]}${unit} to ${d[k][1]}${unit}`,
  }));
}
