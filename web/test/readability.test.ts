import { test } from "node:test";
import assert from "node:assert/strict";
import { moved, regressed } from "../src/readability.ts";
import type { Readability } from "../src/types.ts";

const def = (over: Partial<Readability>): Readability => ({
  name: "f",
  nesting: [1, 1],
  lines: [4, 4],
  params: [1, 1],
  branches: [1, 1],
  cognitive: [1, 1],
  short_names: [0, 0],
  ...over,
});

test("only the measurements that moved are said, in the engine's order", () => {
  assert.deepEqual(moved(def({ lines: [4, 6], nesting: [1, 3], short_names: [50, 0] })), [
    "nesting 1 → 3",
    "short names 50% → 0%",
    "lines 4 → 6",
  ]);
});

test("growing longer alone, or getting simpler, is not a regression", () => {
  assert.equal(regressed(def({ lines: [4, 40] })), false);
  assert.equal(regressed(def({ nesting: [3, 1] })), false);
  assert.equal(regressed(def({ cognitive: [1, 2] })), true);
});
