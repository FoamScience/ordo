import { test } from "node:test";
import assert from "node:assert/strict";
import { centerScroll, codeRows, firstRowOf, runs, tableLimit, wordMove } from "../src/layout.ts";
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
  assert.equal(tableLimit(10, false), 3);
  assert.equal(tableLimit(2, false), 2);
  assert.equal(tableLimit(10, true), 10);
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

test("w skips the word, then the space after it, and crosses rows", () => {
  const lines = ["let a = b;", "  call(x)"];
  assert.deepEqual(wordMove(lines, { row: 0, col: 0 }, "next"), { row: 0, col: 4 });
  assert.deepEqual(wordMove(lines, { row: 0, col: 9 }, "next"), { row: 1, col: 2 });
});

test("b mirrors w, and e lands on a word's last character", () => {
  const lines = ["let a = b;", "  call(x)"];
  assert.deepEqual(wordMove(lines, { row: 1, col: 2 }, "prev"), { row: 0, col: 9 });
  assert.deepEqual(wordMove(lines, { row: 0, col: 4 }, "prev"), { row: 0, col: 0 });
  assert.deepEqual(wordMove(lines, { row: 1, col: 2 }, "end"), { row: 1, col: 5 });
});

test("a caret counts characters, not UTF-16 units", () => {
  assert.deepEqual(wordMove(["é😀 x"], { row: 0, col: 0 }, "next"), { row: 0, col: 1 });
  assert.deepEqual(wordMove(["😀😀 x"], { row: 0, col: 0 }, "end"), { row: 0, col: 1 });
});

test("a line that only gained text reads once, its insertion marked", () => {
  const rows = codeRows(
    file({
      old: ["def side_room(rows):", "    pass"],
      new: ["def side_room(rows, wall=None):", "    return 1"],
      hunks: [{ i: 0, old: [1, 2], new: [1, 2], removed: [[], null], added: [[[18, 29]], null] }],
    }),
  );
  assert.deepEqual(
    rows.map((r) => [r.kind, r.text]),
    [
      ["del", "    pass"],
      ["edit", "def side_room(rows, wall=None):"],
      ["add", "    return 1"],
    ],
  );
  assert.deepEqual(rows[1].marks, [[18, 29]]);
});

test("a line that also lost text stays a removed and an added line", () => {
  const rows = codeRows(
    file({
      old: ["x = f(a)"],
      new: ["x = g(a, b)"],
      hunks: [{ i: 0, old: [1, 1], new: [1, 1], removed: [[[4, 5]]], added: [[[4, 5], [6, 9]]] }],
    }),
  );
  assert.deepEqual(rows.map((r) => r.kind), ["del", "add"]);
});
