import { test } from "node:test";
import assert from "node:assert/strict";
import { centerScroll, codeRows, firstRowOf, runs, tableLimit } from "../src/layout.ts";
import type { FileData } from "../src/types.ts";

test("the selected row sits in the middle of the list", () => {
  // row at 500px, 20px tall, a 200px pane over 2000px of rows
  assert.equal(centerScroll(500, 20, 200, 2000), 410);
});

test("near the ends the list does not scroll past its first or last row", () => {
  assert.equal(centerScroll(40, 20, 200, 2000), 0);
  assert.equal(centerScroll(1980, 20, 200, 2000), 1800);
  assert.equal(centerScroll(10, 20, 200, 100), 0);
});

test("a why table shows three rows until the pane is maximised", () => {
  assert.equal(tableLimit(10, false, 20), 3);
  assert.equal(tableLimit(2, false, 20), 2);
  assert.equal(tableLimit(10, true, 7), 7);
  assert.equal(tableLimit(10, true, 40), 10);
  assert.equal(tableLimit(10, true, 1), 3);
});

const file = (over: Partial<FileData>): FileData => ({
  path: "a.py",
  old: [],
  new: [],
  old_spans: null,
  new_spans: null,
  hunks: [],
  comments: [],
  ...over,
});

test("a removal stands above the line that followed it, and its hunk opens on it", () => {
  const f = file({
    old: ["a", "gone1", "gone2", "b"],
    new: ["a", "b"],
    hunks: [{ i: 7, old: [2, 3], new: [2, 1], removed: [null, null], added: [] }],
  });
  const rows = codeRows(f);
  assert.deepEqual(
    rows.map((r) => `${r.kind}:${r.text}`),
    ["ctx:a", "del:gone1", "del:gone2", "ctx:b"],
  );
  assert.equal(firstRowOf(rows, 7), 1, "the first removed line, not the last");
});

test("an edit reads removed lines first, then the added ones, refined spans kept", () => {
  const f = file({
    old: ["x = 1"],
    new: ["x = 2"],
    hunks: [{ i: 0, old: [1, 1], new: [1, 1], removed: [[[4, 5]]], added: [[[4, 5]]] }],
  });
  const rows = codeRows(f);
  assert.deepEqual(rows.map((r) => r.kind), ["del", "add"]);
  assert.deepEqual(rows[1].marks, [[4, 5]]);
});

test("a deletion at the end of the file still shows", () => {
  const f = file({ old: ["a", "z"], new: ["a"], hunks: [{ i: 1, old: [2, 2], new: [2, 1], removed: [null], added: [] }] });
  assert.deepEqual(codeRows(f).map((r) => r.kind), ["ctx", "del"]);
});

test("a line splits into runs by syntax role and by what changed", () => {
  const out = runs("x = 22", [["x", "variable"], [" = ", "operator"], ["22", "number"]], [[5, 6]]);
  assert.deepEqual(out, [
    { text: "x", role: "variable", changed: false },
    { text: " = ", role: "operator", changed: false },
    { text: "2", role: "number", changed: false },
    { text: "2", role: "number", changed: true },
  ]);
});
