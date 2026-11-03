// Geometry that has a right answer, kept apart from the DOM so it can be
// tested: where the list scrolls, how many table rows show, what the code
// pane's rows are.

import DiffMatchPatch from "diff-match-patch";
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
  | {
      kind: "ctx" | "add" | "edit";
      n: number;
      text: string;
      spans: Spans | null;
      hunk: number | null;
      marks: [number, number][] | null;
      /** on an `edit`, the text the old line had that this one lost, by the column it stood before */
      gone?: [number, string][];
    }
  | { kind: "del"; o: number; text: string; spans: Spans | null; hunk: number; marks: [number, number][] | null };

/**
 * The file as the code pane reads it: the new side top to bottom, each hunk's
 * removed lines standing just above where its new lines begin. A pure
 * deletion's lines stand above the line that followed them. A line edited in
 * place is one `edit` row — the new line, what it gained marked and what it
 * lost struck through where it stood — not a removed line and an added one
 * that say most of the same thing twice.
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
  const { folded, edited } = editsInPlace(f, hunks);
  const removedBefore = (n: number) => {
    for (const h of startingAt.get(n) ?? []) {
      for (let o = h.old[0]; o <= h.old[1]; o++) {
        if (folded.has(o)) continue;
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
    const e = edited.get(n);
    rows.push({
      kind: a ? (e ? "edit" : "add") : "ctx",
      n,
      text: f.new[n - 1],
      spans: f.new_spans?.[n - 1] ?? null,
      hunk: a?.hunk ?? null,
      marks: e ? e.marks : (a?.marks ?? null),
      ...(e && e.gone.length ? { gone: e.gone } : {}),
    });
  }
  removedBefore(f.new.length + 1);
  return rows;
}

interface Edit {
  marks: [number, number][];
  gone: [number, string][];
}

/**
 * Removed lines that are an added line edited in place, paired in order. The
 * refiner's pairing and spans come first; a pair it left whole (a docstring
 * is one leaf, so its lines never pair), or marked with spans that do not
 * line up (`errors,` added without the space after it), is diffed by
 * character, and kept as an edit only when most of the line survived.
 */
export function editsInPlace(f: FileData, hunks: FileData["hunks"]): { folded: Set<number>; edited: Map<number, Edit> } {
  const folded = new Set<number>();
  const edited = new Map<number, Edit>();
  for (const h of hunks) {
    let next = h.new[0];
    for (let o = h.old[0]; o <= h.old[1]; o++) {
      const rm = h.removed[o - h.old[0]];
      const oldText = f.old[o - 1] ?? "";
      for (let n = next; n <= h.new[1]; n++) {
        if (edited.has(n)) continue;
        const add = h.added[n - h.new[0]];
        const newText = f.new[n - 1] ?? "";
        const e = (rm && add ? fromSpans(oldText, newText, rm, add) : null) ?? (!rm === !add ? byChars(oldText, newText) : null);
        if (e) {
          folded.add(o);
          edited.set(n, e);
          next = n + 1;
        }
        break;
      }
    }
  }
  return { folded, edited };
}

/** The refiner's spans as an edit, when what neither side marked reads the same. */
function fromSpans(oldText: string, newText: string, rm: [number, number][], add: [number, number][]): Edit | null {
  const o = [...oldText];
  const n = [...newText];
  const inside = (spans: [number, number][], k: number) => spans.some(([a, b]) => k >= a && k < b);
  const gone: [number, string][] = [];
  let i = 0;
  let j = 0;
  while (i < o.length || j < n.length) {
    if (i < o.length && inside(rm, i)) {
      const from = i;
      while (i < o.length && inside(rm, i)) i++;
      gone.push([j, o.slice(from, i).join("")]);
    } else if (j < n.length && inside(add, j)) {
      j++;
    } else if (i < o.length && j < n.length && o[i] === n[j]) {
      i++;
      j++;
    } else {
      return null;
    }
  }
  return { marks: add, gone };
}

const dmp = new DiffMatchPatch();

/** How much of a line must survive for an unpaired pair to read as one line edited. */
const KEPT = 0.6;

/** A character diff of two lines as an edit, or null when they share too little. */
export function byChars(oldText: string, newText: string): Edit | null {
  if (!oldText.trim() || !newText.trim()) return null;
  const diffs = dmp.diff_main(oldText, newText);
  dmp.diff_cleanupSemantic(diffs);
  const marks: [number, number][] = [];
  const gone: [number, string][] = [];
  let col = 0;
  let kept = 0;
  for (const [op, text] of diffs) {
    const len = [...text].length;
    if (op === DiffMatchPatch.DIFF_EQUAL) {
      kept += len;
      col += len;
    } else if (op === DiffMatchPatch.DIFF_INSERT) {
      marks.push([col, col + len]);
      col += len;
    } else {
      gone.push([col, text]);
    }
  }
  if (kept < KEPT * Math.max([...oldText].length, [...newText].length)) return null;
  // the space beside an inserted word is where the diff cut, not what changed
  const chars = [...newText];
  const trimmed = marks
    .map(([a, b]): [number, number] => {
      while (a < b && /\s/.test(chars[a])) a++;
      while (b > a && /\s/.test(chars[b - 1])) b--;
      return [a, b];
    })
    .filter(([a, b]) => b > a);
  return { marks: trimmed, gone };
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
export function runs(
  text: string,
  spans: Spans | null,
  marks: [number, number][] | null,
  gone: [number, string][] = [],
): { text: string; role: string | null; changed: boolean; gone?: true }[] {
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
  const out: { text: string; role: string | null; changed: boolean; gone?: true }[] = [];
  const lost = (k: number) => {
    for (const [at, t] of gone) if (at === k) out.push({ text: t, role: null, changed: false, gone: true });
  };
  for (let k = 0; k < chars.length; k++) {
    lost(k);
    const last = out[out.length - 1];
    if (last && !last.gone && last.role === role[k] && last.changed === changed[k]) last.text += chars[k];
    else out.push({ text: chars[k], role: role[k], changed: changed[k] });
  }
  lost(chars.length);
  return out;
}

export interface Caret {
  row: number;
  col: number;
}

// `w`, `b` and `e` as the terminal moves them: over the rows joined by "\n",
// so a word that ends a row and one that starts the next are two words, the
// same as across a space. Columns count characters, not UTF-16 units.
function wordClass(c: string): number {
  if (/\s/u.test(c)) return 0;
  return /[\p{L}\p{N}_]/u.test(c) ? 1 : 2;
}

function flatten(lines: string[]): { chars: string[]; starts: number[] } {
  const chars: string[] = [];
  const starts: number[] = [];
  lines.forEach((l, k) => {
    starts.push(chars.length);
    chars.push(...l);
    if (k + 1 < lines.length) chars.push("\n");
  });
  return { chars, starts };
}

function caretAt(lines: string[], starts: number[], i: number): Caret {
  let row = 0;
  while (row + 1 < starts.length && starts[row + 1] <= i) row++;
  return { row, col: Math.min(i - starts[row], [...lines[row]].length) };
}

/** Where `w` (`next`), `b` (`prev`) or `e` (`end`) takes the caret. */
export function wordMove(lines: string[], at: Caret, kind: "next" | "prev" | "end"): Caret {
  const { chars, starts } = flatten(lines);
  if (!chars.length) return { row: 0, col: 0 };
  let i = Math.min((starts[at.row] ?? 0) + at.col, chars.length - 1);
  const cls = (k: number) => wordClass(chars[k]);
  if (kind === "next") {
    const c = cls(i);
    if (c !== 0) while (i + 1 < chars.length && cls(i + 1) === c) i++;
    while (i + 1 < chars.length && cls(i + 1) === 0) i++;
    return caretAt(lines, starts, Math.min(i + 1, chars.length - 1));
  }
  if (kind === "prev") {
    if (i === 0) return { row: 0, col: 0 };
    i--;
    while (i > 0 && cls(i) === 0) i--;
    const c = cls(i);
    while (i > 0 && cls(i - 1) === c) i--;
    return caretAt(lines, starts, i);
  }
  if (i + 1 >= chars.length) return caretAt(lines, starts, i);
  i++;
  while (i < chars.length && cls(i) === 0) i++;
  if (i >= chars.length) return caretAt(lines, starts, chars.length - 1);
  const c = cls(i);
  while (i + 1 < chars.length && cls(i + 1) === c) i++;
  return caretAt(lines, starts, i);
}

/** The identifier holding column `col` of `text`, with the column it starts at. */
export function wordAt(text: string, col: number): { word: string; start: number } | null {
  const cs = [...text];
  const isWord = (c: string | undefined) => !!c && /\w/.test(c);
  if (!isWord(cs[col])) return null;
  let a = col;
  while (isWord(cs[a - 1])) a--;
  let b = col;
  while (isWord(cs[b + 1])) b++;
  const word = cs.slice(a, b + 1).join("");
  return /^[A-Za-z_]/.test(word) ? { word, start: a } : null;
}

export type StripKind = "mine" | "other" | "verdict" | "warn" | "comment" | "hit";

/** A run of rows that one mark covers in the strip beside the code. */
export interface StripMark {
  kind: StripKind;
  from: number;
  to: number;
}

/**
 * What the strip beside the code marks, as runs of row indexes: the selected
 * hunk's changes, other hunks' changes, then findings, comments and search hits
 * on the lines that carry them.
 */
export function stripMarks(
  rows: CodeRow[],
  sel: number,
  findings: Map<number, "verdict" | "warn">,
  comments: Set<number>,
  hits: number[],
): StripMark[] {
  const out: StripMark[] = [];
  const last = new Map<StripKind, StripMark>();
  const add = (kind: StripKind, k: number) => {
    const m = last.get(kind);
    if (m && m.to === k - 1) m.to = k;
    else {
      const next = { kind, from: k, to: k };
      out.push(next);
      last.set(kind, next);
    }
  };
  rows.forEach((row, k) => {
    if (row.kind !== "ctx" && row.hunk !== null) add(row.hunk === sel ? "mine" : "other", k);
  });
  rows.forEach((row, k) => {
    if (row.kind === "del") return;
    const level = findings.get(row.n);
    if (level) add(level, k);
    if (comments.has(row.n)) add("comment", k);
  });
  for (const k of hits) add("hit", k);
  return out;
}
