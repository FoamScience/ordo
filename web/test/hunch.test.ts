import { test } from "node:test";
import assert from "node:assert/strict";
import { badge, contributions, effect, flagBadge } from "../src/hunch.ts";

const g = { noise: 0.7234, top: [["comment", 1.661], ["noise", -1.0204], ["uses", 0]] as [string, number][] };

test("the badge says the guess as a percentage", () => {
  assert.equal(badge(g), "Hunch: 72% skippable");
});

test("each contribution is signed toward skipping", () => {
  assert.deepEqual(contributions(g), ["comment +1.66", "noise −1.02", "uses +0.00"]);
});

test("a moved noise flag gets its own badge", () => {
  assert.equal(flagBadge(g), null);
  assert.equal(flagBadge({ ...g, changed: "unhid" }), "Hunch: shown");
  assert.equal(flagBadge({ ...g, changed: "hid" }), "Hunch: hidden");
});

test("the why pane says what the guess did", () => {
  assert.equal(effect(g), "a trained guess; it changes nothing here");
  assert.match(effect({ ...g, changed: "unhid" }), /so it is shown$/);
  assert.match(effect({ ...g, changed: "hid" }), /hidden like noise$/);
});
