import { test } from "node:test";
import assert from "node:assert/strict";
import { Keymap, keyLabel } from "../src/keys.ts";

const ev = (key: string, mods: Partial<{ ctrlKey: boolean; altKey: boolean; shiftKey: boolean; metaKey: boolean }> = {}) => ({
  key,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  metaKey: false,
  ...mods,
});

test("a key is named the way tui.toml names it", () => {
  assert.equal(keyLabel(ev("j")), "j");
  assert.equal(keyLabel(ev("G", { shiftKey: true })), "G");
  assert.equal(keyLabel(ev("w", { ctrlKey: true })), "C-w");
  assert.equal(keyLabel(ev(" ")), "Space");
  assert.equal(keyLabel(ev("ArrowDown")), "Down");
  assert.equal(keyLabel(ev("F3", { shiftKey: true })), "S-F3");
  assert.equal(keyLabel(ev("Escape")), "Esc");
});

test("a lone modifier or a command-key chord is no key", () => {
  assert.equal(keyLabel(ev("Shift", { shiftKey: true })), null);
  assert.equal(keyLabel(ev("c", { metaKey: true })), null);
});

test("a chord prefix waits, and a prefix that is also a key keeps its own action", () => {
  const km = new Keymap([
    { prefix: null, key: "]", action: "mark-next" },
    { prefix: "]", key: "f", action: "finding-next" },
    { prefix: "g", key: "g", action: "first" },
    { prefix: null, key: "j", action: "next" },
  ]);
  assert.deepEqual(km.resolve(null, "]"), { pending: true });
  assert.deepEqual(km.resolve("]", "f"), { act: "finding-next" });
  assert.equal(km.bare("]"), "mark-next");
  assert.deepEqual(km.resolve("g", "x"), { miss: true });
  assert.deepEqual(km.resolve(null, "j"), { act: "next" });
  assert.deepEqual(km.keysFor("first"), ["gg"]);
});
