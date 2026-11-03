import { test } from "node:test";
import assert from "node:assert/strict";
import { foldedText, rowText, withSymbols } from "../src/list.ts";
import type { Item, Row } from "../src/types.ts";

const item = (over: Partial<Item>): Item =>
  ({ symbols: [], enclosing: null, rationale: "", details: [], gist: null, ...over }) as Item;

const rows = (...ks: (number | "gap")[]): Row[] => ks.map((k) => (k === "gap" ? { kind: "gap" } : { kind: "item", i: k, opens: true }));

test("two or more hunks in a row about one symbol sit under its name", () => {
  const items = [
    item({ enclosing: "Bezier" }),
    item({ enclosing: "Bezier" }),
    item({ symbols: ["load"], enclosing: "load" }),
    item({ enclosing: "Bezier" }),
  ];
  const { rows: out, runOf } = withSymbols(rows(0, 1, 2, 3), items);
  assert.deepEqual(
    out.map((r) => (r.kind === "symbol" ? `[${r.name}]` : r.kind === "item" ? r.i : r.kind)),
    ["[Bezier]", 0, 1, 2, 3],
  );
  assert.deepEqual(runOf.get(1), [0, 1]);
  assert.equal(runOf.has(3), false);
  assert.equal(out[1].kind === "item" && out[1].opens, true);
});

test("a gap or a nameless hunk ends a run", () => {
  const items = [item({ enclosing: "A" }), item({ enclosing: "A" }), item({}), item({})];
  assert.deepEqual(
    withSymbols(rows(0, "gap", 1, 2, 3), items).rows.map((r) => r.kind),
    ["item", "gap", "item", "item", "item"],
  );
});

test("under a heading a row says what no neighbour says, down to its code", () => {
  const a = item({ enclosing: "B", rationale: "edits B", details: ["adds default to Field"], gist: "a: int = Field(default=1)" });
  const b = item({ enclosing: "B", rationale: "edits B", details: ["adds json_schema_extra to Field"], gist: "b: int = Field(x=1)" });
  const c = item({ enclosing: "B", rationale: "edits B", details: ["adds json_schema_extra to Field"], gist: "c: int = Field(x=2)" });
  const d = item({ enclosing: "B", rationale: "adds argument fast to B", gist: "def B(fast):" });
  const run = [a, b, c, d];
  assert.deepEqual(rowText(a, run), { text: "adds default to Field", code: false });
  assert.deepEqual(rowText(b, run), { text: "b: int = Field(x=1)", code: true });
  assert.deepEqual(rowText(d, run), { text: "adds argument fast", code: false });
  assert.deepEqual(rowText(a, null), { text: "edits B", code: false });
});

test("under a heading a row drops the heading's name from its sentence", () => {
  const a = item({ enclosing: "get_wire", rationale: "adds local seen (2 uses)" });
  const b = item({ enclosing: "get_wire", rationale: "adds 7 lines to get_wire" });
  assert.deepEqual(rowText(b, [a, b]), { text: "adds 7 lines", code: false });
  assert.deepEqual(rowText(a, [a, b]), { text: "adds local seen (2 uses)", code: false });
});

test("a folded row says what it hides", () => {
  const imp = item({ category: "import", rationale: "changes import errors, lazyloader" });
  const fmt = item({ category: "other", rationale: "formatting only" });
  assert.equal(foldedText([imp]), "changes import errors, lazyloader");
  assert.equal(foldedText([imp, imp, fmt]), "3 quiet hunks: 2 imports, 1 formatting or generated");
});
