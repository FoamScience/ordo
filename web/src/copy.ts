// What the code pane puts on the clipboard: the new side's text, as the
// file has it. The rows' gutter, findings, comments and struck-through old
// text are never part of it.

import type { CodeRow } from "./layout.ts";

/** A row, cut to the part of its text that was selected. */
export interface Piece {
  kind: CodeRow["kind"];
  text: string;
}

/** The part of a row between two columns of its text: all of it by default. */
export function cutRow(row: Pick<CodeRow, "kind" | "text">, from = 0, to = row.text.length): Piece {
  return { kind: row.kind, text: row.text.slice(from, to) };
}

/**
 * The pieces joined by line breaks. Deleted rows are the old side, so they
 * are left out unless the selection holds nothing else.
 */
export function joinPieces(pieces: Piece[]): string {
  const live = pieces.filter((p) => p.kind !== "del");
  return (live.length ? live : pieces).map((p) => p.text).join("\n");
}

/** Whole rows, for the keys: `y` on the picked lines, `Y` on a hunk. */
export function rowsText(rows: Pick<CodeRow, "kind" | "text">[]): string {
  return joinPieces(rows.map((r) => ({ kind: r.kind, text: r.text })));
}

/** The rows `Y` copies: the hunk's, whole, however far the cursor has strayed from it. */
export function hunkRows<R extends Pick<CodeRow, "kind" | "hunk">>(rows: R[], hunk: number): R[] {
  return rows.filter((r) => r.hunk === hunk);
}

/** The rows `y` copies: the picked lines, or the cursor's own. */
export function pickedRows<R>(rows: R[], cursor: number, from: number | null): R[] {
  const a = Math.min(cursor, from ?? cursor);
  const b = Math.max(cursor, from ?? cursor);
  return rows.slice(a, b + 1);
}

/**
 * Writes `text` to the clipboard. A browser that refuses (no secure context,
 * no permission) gets `refused` instead, so the text is never lost.
 */
export async function toClipboard(
  write: (text: string) => Promise<void>,
  text: string,
  copied: () => void,
  refused: () => void,
): Promise<void> {
  try {
    await write(text);
  } catch {
    return refused();
  }
  copied();
}
