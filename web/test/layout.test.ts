import { test } from "node:test";
import assert from "node:assert/strict";
import { byChars, centerScroll, codeRows, firstRowOf, runs, stripMarks, tableLimit, wordAt, wordMove } from "../src/layout.ts";
import type { FileData } from "../src/types.ts";
import type { CodeRow } from "../src/layout.ts";

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
    old: ["x = 1", "gone()"],
    new: ["x = 2", "y = 3"],
    hunks: [{ i: 0, old: [1, 2], new: [1, 2], removed: [[[4, 5]], null], added: [[[4, 5]], null] }],
  });
  const rows = codeRows(f);
  assert.deepEqual(rows.map((r) => r.kind), ["del", "edit", "add"]);
  assert.deepEqual(rows[1].marks, [[4, 5]]);
});

test("a name added to an import reads as one edited line, even when the refiner's span stops short", () => {
  const f = file({
    old: ["from geolib import lazyloader"],
    new: ["from geolib import errors, lazyloader"],
    // the refiner marks `errors,` and leaves the space after it unmarked
    hunks: [{ i: 0, old: [1, 1], new: [1, 1], removed: [[]], added: [[[19, 26]]] }],
  });
  const rows = codeRows(f);
  assert.deepEqual(rows.map((r) => r.kind), ["edit"]);
  assert.deepEqual(rows[0].marks, [[19, 26]]);
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

test("moved text is its own run, apart from what merely changed", () => {
  const out = runs("f(b, a)", null, [[2, 6]], [], [[2, 3]]);
  assert.deepEqual(out, [
    { text: "f(", role: null, changed: false },
    { text: "b", role: null, changed: true, moved: true },
    { text: ", a", role: null, changed: true },
    { text: ")", role: null, changed: false },
  ]);
});

test("a hunk's moved spans reach the rows they belong to", () => {
  const f = file({
    old: ["a", "m", "b"],
    new: ["a", "b", "m"],
    hunks: [{ i: 1, old: [2, 3], new: [2, 3], removed: [null, [[0, 1]]], added: [[[0, 1]], null], moved_removed: [[[0, 1]], []], moved_added: [[], [[0, 1]]] }],
  });
  const rows = codeRows(f);
  assert.deepEqual(rows.find((r) => r.kind === "del" && r.text === "m")?.moved, [[0, 1]]);
  assert.deepEqual(rows.find((r) => r.kind !== "del" && r.text === "m")?.moved, [[0, 1]]);
  assert.equal(rows.find((r) => r.kind !== "del" && r.text === "b")?.moved, undefined);
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

test("a line that also lost text reads once, the lost text where it stood", () => {
  const rows = codeRows(
    file({
      old: ["x = f(a)"],
      new: ["x = g(a, b)"],
      hunks: [{ i: 0, old: [1, 1], new: [1, 1], removed: [[[4, 5]]], added: [[[4, 5], [7, 10]]] }],
    }),
  );
  assert.deepEqual(rows.map((r) => r.kind), ["edit"]);
  const row = rows[0] as Exclude<CodeRow, { kind: "del" }>;
  assert.deepEqual(row.gone, [[4, "f"]]);
  assert.deepEqual(
    runs(row.text, null, row.marks, row.gone).map((r) => [r.text, !!r.changed, !!r.gone]),
    [["x = ", false, false], ["f", false, true], ["g", true, false], ["(a", false, false], [", b", true, false], [")", false, false]],
  );
});

test("lines the refiner left whole are diffed by character when most survived", () => {
  const was = "    ``build_impeller_body`` runs it out to the hub outlet (``_back_to``).";
  const now = "    ``build_impeller_body`` runs it out to the hub outlet (``back_to``).";
  assert.deepEqual(byChars(was, now), { marks: [], gone: [[61, "_"]] });
  const rows = codeRows(file({ old: [was], new: [now], hunks: [{ i: 0, old: [1, 1], new: [1, 1], removed: [null], added: [null] }] }));
  assert.deepEqual(rows.map((r) => r.kind), ["edit"]);
  // two lines that share little stay apart
  assert.equal(byChars("return compute(a)", "raise ValueError('no')"), null);
});

test("the word under the pointer is the whole identifier, and only an identifier", () => {
  assert.deepEqual(wordAt("x = side_room(rows)", 7), { word: "side_room", start: 4 });
  assert.deepEqual(wordAt("x = side_room(rows)", 4), { word: "side_room", start: 4 });
  assert.equal(wordAt("x = side_room(rows)", 13), null);
  assert.equal(wordAt("n = 42", 4), null);
});

test("the strip marks runs of changed rows, then what sits on the lines", () => {
  const row = (kind: "ctx" | "add" | "del", n: number, hunk: number | null): CodeRow =>
    kind === "del"
      ? { kind, o: n, text: "", spans: null, hunk: hunk!, marks: null }
      : { kind, n, text: "", spans: null, hunk, marks: null };
  const rows = [row("ctx", 1, null), row("del", 2, 0), row("add", 2, 0), row("add", 3, 0), row("ctx", 4, null), row("add", 5, 1)];
  assert.deepEqual(stripMarks(rows, 0, new Map([[3, "warn"]]), new Set([4, 5]), [0]), [
    { kind: "mine", from: 1, to: 3 },
    { kind: "other", from: 5, to: 5 },
    { kind: "warn", from: 3, to: 3 },
    { kind: "comment", from: 4, to: 5 },
    { kind: "hit", from: 0, to: 0 },
  ]);
});
