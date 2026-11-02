// The review on one page: what was read, what was written, what ordo flagged.

import type { Finding, Level, LineComment, Review } from "./types.ts";

export interface FileLine {
  path: string;
  hunks: number[];
  read: number;
  comments: number;
  levels: Partial<Record<Level, number>>;
}

/** Each file in reading order, with how far it has been read. */
export function files(r: Review): FileLine[] {
  const by = new Map<string, FileLine>();
  for (const i of r.view) {
    const it = r.items[i];
    const f = by.get(it.path) ?? { path: it.path, hunks: [], read: 0, comments: 0, levels: {} };
    f.hunks.push(i);
    if (it.reviewed) f.read++;
    for (const x of it.findings) f.levels[x.level] = (f.levels[x.level] ?? 0) + 1;
    by.set(it.path, f);
  }
  for (const c of r.comments) {
    const f = by.get(c.path);
    if (f) f.comments++;
  }
  return [...by.values()];
}

/** The hunk in the review whose new lines hold `line`, else the file's nearest one above it. */
export function hunkAt(r: Review, path: string, line: number): number | null {
  let best: number | null = null;
  for (const i of r.view) {
    const it = r.items[i];
    if (it.path !== path) continue;
    if (it.new[0] <= line && line <= it.new[1]) return i;
    if (it.new[0] <= line && (best === null || it.new[0] > r.items[best].new[0])) best = i;
  }
  return best;
}

export type Written =
  | { kind: "note"; i: number; text: string }
  | { kind: "comment"; i: number | null; comment: LineComment & { path: string } };

/**
 * The reviewer's notes and line comments, file by file in reading order. A
 * note belongs to a symbol, so it is listed once, at the symbol's first hunk.
 */
export function written(r: Review): Written[] {
  const out: { at: [number, number]; w: Written }[] = [];
  const pos = new Map(r.view.map((i, k) => [i, k]));
  const fileAt = new Map<string, number>();
  for (const i of r.view) if (!fileAt.has(r.items[i].path)) fileAt.set(r.items[i].path, pos.get(i)!);
  const seen = new Set<string>();
  for (const i of r.view) {
    const it = r.items[i];
    const key = `${it.symbol || it.symbols[0] || i}\u0000${it.note}`;
    if (!it.note || seen.has(key)) continue;
    seen.add(key);
    out.push({ at: [fileAt.get(it.path)!, it.new[0]], w: { kind: "note", i, text: it.note } });
  }
  for (const c of r.comments) {
    const file = fileAt.get(c.path);
    if (file === undefined) continue;
    out.push({ at: [file, c.start], w: { kind: "comment", i: hunkAt(r, c.path, c.start), comment: c } });
  }
  return out.sort((a, b) => a.at[0] - b.at[0] || a.at[1] - b.at[1]).map((x) => x.w);
}

/** Findings at `levels`, most serious first, each with the hunk it sits in. */
export function flagged(r: Review, levels: Level[]): { i: number; f: Finding }[] {
  const out = r.view.flatMap((i) => r.items[i].findings.filter((f) => levels.includes(f.level)).map((f) => ({ i, f })));
  return out.sort((a, b) => levels.indexOf(a.f.level) - levels.indexOf(b.f.level));
}
