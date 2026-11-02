import { test } from "node:test";
import assert from "node:assert/strict";
import { files, flagged, hunkAt, written } from "../src/summary.ts";
import type { Finding, Item, Review } from "../src/types.ts";

const item = (path: string, from: number, to: number, over: Partial<Item> = {}): Item =>
  ({ path, new: [from, to], reviewed: false, findings: [], note: null, symbol: null, symbols: [], ...over }) as Item;

const finding = (level: Finding["level"], name: string): Finding => ({ source: "ordo", name, message: name, level });

const review = (items: Item[], over: Partial<Review> = {}): Review =>
  ({ items, view: items.map((_, i) => i), comments: [], ...over }) as Review;

test("files follow the reading order and count what was read, commented and flagged", () => {
  const r = review(
    [
      item("b.py", 1, 5, { reviewed: true, findings: [finding("warn", "w")] }),
      item("a.py", 1, 5),
      item("b.py", 10, 12, { findings: [finding("warn", "w"), finding("verdict", "v")] }),
    ],
    { comments: [{ path: "b.py", start: 3, end: 3, text: "why", stale: false }] },
  );
  assert.deepEqual(files(r), [
    { path: "b.py", hunks: [0, 2], read: 1, comments: 1, levels: { warn: 2, verdict: 1 } },
    { path: "a.py", hunks: [1], read: 0, comments: 0, levels: {} },
  ]);
});

test("a comment belongs to the hunk holding its line, else the nearest one above it", () => {
  const r = review([item("a.py", 1, 5), item("a.py", 20, 30), item("b.py", 1, 9)]);
  assert.equal(hunkAt(r, "a.py", 25), 1);
  assert.equal(hunkAt(r, "a.py", 12), 0);
  assert.equal(hunkAt(r, "b.py", 40), 2);
  assert.equal(hunkAt(r, "c.py", 1), null);
});

test("what was written reads file by file, and a symbol's note is listed once", () => {
  const r = review(
    [
      item("b.py", 10, 12, { symbol: "f", note: "rename it" }),
      item("a.py", 1, 5),
      item("b.py", 20, 22, { symbol: "f", note: "rename it" }),
    ],
    {
      comments: [
        { path: "a.py", start: 2, end: 2, text: "a", stale: false },
        { path: "b.py", start: 15, end: 16, text: "b", stale: true },
        { path: "gone.py", start: 1, end: 1, text: "elsewhere", stale: false },
      ],
    },
  );
  const w = written(r);
  assert.deepEqual(
    w.map((x) => (x.kind === "note" ? `note ${x.i}` : `comment ${x.comment.text} in ${x.i}`)),
    ["note 0", "comment b in 0", "comment a in 1"],
  );
});

test("flagged findings come most serious first, at the asked levels only", () => {
  const r = review([item("a.py", 1, 2, { findings: [finding("warn", "w"), finding("note", "n")] }), item("a.py", 5, 6, { findings: [finding("verdict", "v")] })]);
  assert.deepEqual(
    flagged(r, ["verdict", "warn"]).map((x) => `${x.f.name}@${x.i}`),
    ["v@1", "w@0"],
  );
});

test("notes on hunks with no group name are not merged because their text matches", () => {
  const r = review([item("a.py", 1, 2, { symbol: "", note: "check this" }), item("a.py", 9, 10, { symbol: "", note: "check this" })]);
  assert.equal(written(r).length, 2);
});
