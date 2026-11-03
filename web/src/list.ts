// The reading order as the list draws it: consecutive hunks about one symbol
// sit under its name, each said by what sets it apart from its neighbours.

import type { Item, Row } from "./types.ts";

export type ListRow = Row | { kind: "symbol"; name: string; items: number[] };

/** The name a hunk is about: what it declares, else what it sits in. */
export function aboutOf(it: Item): string | null {
  return it.symbols[0] || it.enclosing || null;
}

/** `rows` with a symbol heading before each run of hunks about one name, a single hunk included. */
export function withSymbols(rows: Row[], items: Item[]): { rows: ListRow[]; runOf: Map<number, number[]> } {
  const out: ListRow[] = [];
  const runOf = new Map<number, number[]>();
  let run: number[] = [];
  const flush = () => {
    const name = run.length ? aboutOf(items[run[0]]) : null;
    if (name) {
      out.push({ kind: "symbol", name, items: run });
      for (const i of run) runOf.set(i, run);
    }
    for (const i of run) out.push({ kind: "item", i, opens: false });
    run = [];
  };
  for (const row of rows) {
    if (row.kind === "item") {
      const about = aboutOf(items[row.i]);
      if (run.length && (!about || about !== aboutOf(items[run[0]]))) flush();
      run.push(row.i);
      continue;
    }
    flush();
    out.push(row);
  }
  flush();
  // the rows put back are the same items; keep what the server said about each
  const opens = new Map(rows.flatMap((r) => (r.kind === "item" ? [[r.i, r.opens] as const] : [])));
  return { rows: out.map((r) => (r.kind === "item" ? { ...r, opens: opens.get(r.i) ?? false } : r)), runOf };
}

/**
 * What a row says. Alone, its sentence. Under a symbol heading, the first of
 * these that no neighbour shares: its sentence without the heading's name,
 * unless nothing but the verb is left; the engine's detail; then the code it
 * changes.
 */
export function rowText(it: Item, run: Item[] | null): { text: string; code: boolean } {
  if (!run) return { text: it.rationale, code: false };
  const alone = (pick: (o: Item) => string | undefined) => {
    const v = pick(it);
    return v && run.filter((o) => pick(o) === v).length === 1 ? v : null;
  };
  const name = aboutOf(it) ?? "";
  const sentence = alone((o) => o.rationale);
  const tail = new RegExp(`\\s+(?:to|in|of|from|on)\\s+${name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}$`);
  const short = sentence?.replace(tail, "");
  if (short && short !== `edits ${name}`) return { text: short, code: false };
  const detail = alone((o) => o.details[0]);
  if (detail) return { text: detail, code: false };
  if (it.gist) return { text: it.gist, code: true };
  return { text: it.details[0] ?? it.rationale, code: false };
}

/** A folded row of quiet hunks: one says its own sentence, several say what they are. */
export function foldedText(items: Item[]): string {
  if (items.length === 1) return items[0].rationale;
  const imports = items.filter((it) => it.category === "import").length;
  const rest = items.length - imports;
  const parts = [imports && `${imports} import${imports === 1 ? "" : "s"}`, rest && `${rest} formatting or generated`].filter(Boolean);
  return `${items.length} quiet hunks: ${parts.join(", ")}`;
}
