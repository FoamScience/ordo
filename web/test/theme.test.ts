import { test } from "node:test";
import assert from "node:assert/strict";
import { baseOf, cssVar } from "../src/theme.ts";

test("ordo's theme roles land on this page's variables", () => {
  assert.equal(cssVar("syntax-keyword"), "--syn-keyword");
  assert.equal(cssVar("border-focus"), "--focus");
  assert.equal(cssVar("verdict-strong-bg"), "--verdict-strong-bg");
  assert.equal(cssVar("fg"), "--fg");
  assert.equal(cssVar("warn"), "--warn");
});

test("a theme with light text sits on a dark base", () => {
  assert.equal(baseOf({ fg: "#cdd6f4" }), "dark");
  assert.equal(baseOf({ fg: "#4c4f69" }), "light");
  assert.equal(baseOf({ fg: null }), null);
});
