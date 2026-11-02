// Geometry that has a right answer, kept apart from the DOM so it can be
// tested: where the list scrolls, how many table rows show, what the code
// pane's rows are.

import type { FileData, Spans } from "./types.ts";

/**
 * The list's scroll offset that puts a row in the middle of the pane — except
 * at the ends, where centring would scroll past the first or last row.
 */
export function centerScroll(rowTop: number, rowHeight: number, viewHeight: number, contentHeight: number): number {
  const want = rowTop + rowHeight / 2 - viewHeight / 2;
  const max = Math.max(0, contentHeight - viewHeight);
  return Math.round(Math.min(max, Math.max(0, want)));
}

/** A why-pane table shows three rows, and every row once the pane is maximised. */
export function tableLimit(total: number, maximised: boolean): number {
  return maximised ? total : Math.min(total, 3);
}

export type CodeRow =
  | { kind: "ctx" | "add"; n: number; text: string; spans: Spans | null; hunk: number | null; marks: [number, number][] | null }
  | { kind: "del"; o: number; text: string; spans: Spans | null; hunk: number; marks: [number, number][] | null };

/**
 * The file as the code pane reads it: the new side top to bottom, each hunk's
 * removed lines standing just above where its new lines begin. A pure
 * deletion's lines stand above the line that followed them.
 */
export function codeRows(f: FileData): CodeRow[] {
  const hunks = [...f.hunks].sort((a, b) => a.new[0] - b.new[0] || a.old[0] - b.old[0]);
  const rows: CodeRow[] = [];
  const startingAt = new Map<number, typeof hunks>();
  for (const h of hunks) startingAt.set(h.new[0], [...(startingAt.get(h.new[0]) ?? []), h]);
  const added = new Map<number, { hunk: number; marks: [number, number][] | null }>();
  for (const h of hunks) {
    for (let n = h.new[0]; n <= h.new[1]; n++) {
      added.set(n, { hunk: h.i, marks: h.added[n - h.new[0]] ?? null });
    }
  }
  const removedBefore = (n: number) => {
    for (const h of startingAt.get(n) ?? []) {
      for (let o = h.old[0]; o <= h.old[1]; o++) {
        rows.push({
          kind: "del",
          o,
          text: f.old[o - 1] ?? "",
          spans: f.old_spans?.[o - 1] ?? null,
          hunk: h.i,
          marks: h.removed[o - h.old[0]] ?? null,
        });
      }
    }
  };
  for (let n = 1; n <= f.new.length; n++) {
    removedBefore(n);
    const a = added.get(n);
    rows.push({
      kind: a ? "add" : "ctx",
      n,
      text: f.new[n - 1],
      spans: f.new_spans?.[n - 1] ?? null,
      hunk: a?.hunk ?? null,
      marks: a?.marks ?? null,
    });
  }
  removedBefore(f.new.length + 1);
  return rows;
}

/** The row a hunk's reading starts on: its first removed line, else its first new one. */
export function firstRowOf(rows: CodeRow[], hunk: number): number {
  const k = rows.findIndex((r) => r.hunk === hunk);
  return k < 0 ? 0 : k;
}

/**
 * A line's text cut into runs, each with its syntax role and whether it is
 * part of the refined (actually changed) span.
 */
export function runs(text: string, spans: Spans | null, marks: [number, number][] | null): { text: string; role: string | null; changed: boolean }[] {
  const chars = [...text];
  const role: (string | null)[] = new Array(chars.length).fill(null);
  if (spans) {
    let p = 0;
    for (const [t, r] of spans) {
      for (let k = 0; k < [...t].length && p < chars.length; k++) role[p++] = r;
    }
  }
  const changed: boolean[] = new Array(chars.length).fill(false);
  for (const [a, b] of marks ?? []) for (let k = a; k < b && k < chars.length; k++) changed[k] = true;
  const out: { text: string; role: string | null; changed: boolean }[] = [];
  for (let k = 0; k < chars.length; k++) {
    const last = out[out.length - 1];
    if (last && last.role === role[k] && last.changed === changed[k]) last.text += chars[k];
    else out.push({ text: chars[k], role: role[k], changed: changed[k] });
  }
  return out;
}
