import { test } from "node:test";
import assert from "node:assert/strict";
import { cascadeOf, depMap, groundOf, needPairs, smoothPath } from "../src/depmap.ts";
import type { Item } from "../src/types.ts";

/** Hunks whose `needs` lists what each one needs, by index. */
const review = (needs: number[][], reviewed: number[] = []): Item[] =>
  needs.map((ns, i) => {
    const neededBy = needs.flatMap((other, j) => (other.includes(i) ? [j] : []));
    return {
      edges: [
        ...ns.map((t) => ({ label: `uses ${t}`, target: t, needs: true })),
        ...neededBy.map((t) => ({ label: `used by ${t}`, target: t, needs: false })),
      ],
      reviewed: reviewed.includes(i),
    } as Item;
  });

test("each need is one pair, from the needed hunk to the one needing it", () => {
  const items = review([[], [0], [0, 1]]);
  assert.deepEqual(needPairs(items, [0, 1, 2]), [
    [0, 1],
    [0, 2],
    [1, 2],
  ]);
});

test("a hunk outside the view has no part in the map", () => {
  const items = review([[], [0], [1]]);
  assert.deepEqual(needPairs(items, [0, 2]), []);
});

test("rejecting a hunk strands what needs it, and what needs those", () => {
  const items = review([[], [0], [1], [], [3]]);
  assert.deepEqual([...cascadeOf(items, [0, 1, 2, 3, 4], 0)].sort(), [1, 2]);
  assert.deepEqual([...cascadeOf(items, [0, 1, 2, 3, 4], 2)], []);
});

test("a cycle of needs ends the cascade instead of looping", () => {
  const items = review([[1], [0]]);
  assert.deepEqual([...cascadeOf(items, [0, 1], 0)], [1]);
});

test("what a hunk stands on is everything it needs, and everything those need", () => {
  const items = review([[], [0], [1], [], [3]]);
  assert.deepEqual([...groundOf(items, [0, 1, 2, 3, 4], 2)].sort(), [0, 1]);
  assert.deepEqual([...groundOf(items, [0, 1, 2, 3, 4], 0)], []);
});

test("the chain holds a hunk's ground left of it and its cascade right of it, and nothing else", () => {
  // 0 <- 1 <- 2, 3 unlinked, 4 needs 3
  const items = review([[], [0], [1], [], [3]]);
  const m = depMap(items, [0, 1, 2, 3, 4], 1, { width: 200, height: 40 });
  assert.deepEqual([...m.nodes.keys()].sort(), [0, 1, 2]);
  assert.ok(m.nodes.get(0)!.x < m.nodes.get(1)!.x);
  assert.ok(m.nodes.get(1)!.x < m.nodes.get(2)!.x);
  assert.equal(m.edges.length, 2);
  assert.ok(m.width > 3 * 200 && m.height >= 40);
});

test("a hunk marked reviewed before what it needs is flagged on its edge", () => {
  const m = depMap(review([[], [0]], [1]), [0, 1], 0, { width: 200, height: 40 });
  assert.equal(m.edges[0].outOfOrder, true);
  const fine = depMap(review([[], [0]], [0, 1]), [0, 1], 0, { width: 200, height: 40 });
  assert.equal(fine.edges[0].outOfOrder, false);
});

test("a path rounds its corners and ends on the last point", () => {
  assert.equal(
    smoothPath([
      { x: 0, y: 0 },
      { x: 0, y: 10 },
      { x: 10, y: 10 },
    ]),
    "M 0 0 Q 0 10 5 10 L 10 10",
  );
});
