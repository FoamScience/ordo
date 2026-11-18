import { test } from "node:test";
import assert from "node:assert/strict";
import { badge, contributions } from "../src/hunch.ts";

const g = { noise: 0.7234, top: [["comment", 1.661], ["noise", -1.0204], ["uses", 0]] as [string, number][] };

test("the badge says the guess as a percentage", () => {
  assert.equal(badge(g), "Hunch: 72% skippable");
});

test("each contribution is signed toward skipping", () => {
  assert.deepEqual(contributions(g), ["comment +1.66", "noise −1.02", "uses +0.00"]);
});
