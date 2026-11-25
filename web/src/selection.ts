// The browser's text selection in the code pane, read as rows and columns
// of the file so it can be copied clean and put back after a re-render.
// A row's text is its content span's text without the `del.gone` old text.

import { cutRow, joinPieces, type Piece } from "./copy.ts";
import type { CodeRow } from "./layout.ts";

const GONE = "del.gone";

const contentOf = (line: Element): HTMLElement => line.lastElementChild as HTMLElement;

/** The code units of the row's own text before the point (`node`, `offset`), or 0 / the row's length when the point is before / after the row's text. */
function colAt(content: HTMLElement, node: Node, offset: number, length: number): number {
  const whole = document.createRange();
  whole.selectNodeContents(content);
  const at = whole.comparePoint(node, offset);
  if (at < 0) return 0;
  if (at > 0) return length;
  const head = document.createRange();
  head.setStart(content, 0);
  head.setEnd(node, offset);
  const frag = head.cloneContents();
  frag.querySelectorAll(GONE).forEach((d) => d.remove());
  return frag.textContent!.length;
}

/** Where, relative to the row's text, the range touches it: null when it does not. */
function cut(content: HTMLElement, range: Range, length: number): [number, number] | null {
  const whole = document.createRange();
  whole.selectNodeContents(content);
  if (whole.comparePoint(range.endContainer, range.endOffset) < 0) return null;
  if (whole.comparePoint(range.startContainer, range.startOffset) > 0) return null;
  return [colAt(content, range.startContainer, range.startOffset, length), colAt(content, range.endContainer, range.endOffset, length)];
}

/**
 * The selection's ranges that a copy of the code pane takes over: those of a
 * selection that began in `body`. A part that strays out of the pane (a drag
 * past its edge, or a second range from another pane) is brought back to the
 * pane's edge; a selection that began elsewhere is the browser's to copy.
 */
export function selectedRanges(body: Element): Range[] | null {
  const sel = getSelection();
  if (!sel || sel.isCollapsed || !sel.rangeCount || !sel.anchorNode || !body.contains(sel.anchorNode)) return null;
  const out: Range[] = [];
  for (let i = 0; i < sel.rangeCount; i++) {
    const range = sel.getRangeAt(i).cloneRange();
    const starts = body.contains(range.startContainer);
    const ends = body.contains(range.endContainer);
    if (!starts && !ends) continue;
    if (!starts) range.setStart(body, 0);
    if (!ends) range.setEnd(body, body.childNodes.length);
    out.push(range);
  }
  return out.length ? out : null;
}

/** The text a copy of `ranges` puts on the clipboard; null when they touch no row's text (a finding, a comment), which the browser copies itself. */
export function selectedText(ranges: Range[], body: Element, rows: Pick<CodeRow, "kind" | "text">[]): string | null {
  const pieces: Piece[] = [];
  for (const range of ranges) {
    for (const line of body.querySelectorAll<HTMLElement>(".l")) {
      const row = rows[Number(line.dataset.k)];
      if (!row || !range.intersectsNode(line)) continue;
      const at = cut(contentOf(line), range, row.text.length);
      if (at) pieces.push(cutRow(row, at[0], at[1]));
    }
  }
  return pieces.length ? joinPieces(pieces) : null;
}

/** Where the selection is, to tell a click that ended a drag from one that did not. */
export function markSelection(): unknown[] {
  const sel = getSelection();
  return sel ? [sel.anchorNode, sel.anchorOffset, sel.focusNode, sel.focusOffset] : [];
}

export const sameMark = (a: unknown[], b: unknown[]): boolean => a.length === b.length && a.every((x, i) => x === b[i]);

/** The row's own text as drawn: the content span without the struck-through old text. */
function rowText(line: Element): string {
  const copy = contentOf(line).cloneNode(true) as HTMLElement;
  copy.querySelectorAll(GONE).forEach((d) => d.remove());
  return copy.textContent!;
}

/** One end of a selection: the row, the column in it and the row's text then. */
type End = [number, number, string];

export interface Saved {
  /** the file the rows belonged to */
  path: string;
  anchor: End;
  focus: End;
}

function endOf(body: Element, node: Node | null, offset: number): End | null {
  const line = node && (node instanceof Element ? node : node.parentElement)?.closest<HTMLElement>(".l");
  if (!line || !body.contains(line)) return null;
  const content = contentOf(line);
  if (!content.contains(node)) return null;
  const text = rowText(line);
  return [Number(line.dataset.k), colAt(content, node!, offset, text.length), text];
}

/** The selection as rows and columns of what is drawn now, or null when there is none inside the code. */
export function saveSelection(body: Element, path: string): Saved | null {
  const sel = getSelection();
  if (!sel || sel.isCollapsed) return null;
  const anchor = endOf(body, sel.anchorNode, sel.anchorOffset);
  const focus = endOf(body, sel.focusNode, sel.focusOffset);
  return anchor && focus ? { path, anchor, focus } : null;
}

function pointAt(line: HTMLElement, col: number): [Node, number] {
  const content = contentOf(line);
  let left = col;
  let last: Text | null = null;
  const walk = document.createTreeWalker(content, NodeFilter.SHOW_TEXT, {
    acceptNode: (n) => (n.parentElement?.closest(GONE) ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT),
  });
  for (let n = walk.nextNode() as Text | null; n; n = walk.nextNode() as Text | null) {
    if (left <= n.length) return [n, left];
    left -= n.length;
    last = n;
  }
  return last ? [last, last.length] : [content, 0];
}

/** Puts a saved selection back on a rebuilt pane; false when the rows are no longer the ones it was made on. */
export function restoreSelection(body: Element, saved: Saved, path: string, rows: { text: string }[]): boolean {
  if (saved.path !== path) return false;
  const lines = new Map([...body.querySelectorAll<HTMLElement>(".l")].map((l) => [Number(l.dataset.k), l]));
  for (const [k, , text] of [saved.anchor, saved.focus]) {
    if (!lines.has(k) || rows[k]?.text !== text) return false;
  }
  const [a, f] = [saved.anchor, saved.focus].map(([k, col]) => pointAt(lines.get(k)!, col));
  getSelection()!.setBaseAndExtent(a[0], a[1], f[0], f[1]);
  return true;
}
