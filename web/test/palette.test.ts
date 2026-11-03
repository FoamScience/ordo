import { test } from "node:test";
import assert from "node:assert/strict";
import { complete, hitRow, parseCommand } from "../src/palette.ts";

const cmds = [
  { name: "mode", args: "[ledger|hunks]", help: "" },
  { name: "goto", args: "<path>", help: "" },
  { name: "group", args: "", help: "" },
];

test("a command line splits into name and argument", () => {
  assert.deepEqual(parseCommand(":goto src/a.rs"), { name: "goto", arg: "src/a.rs" });
  assert.deepEqual(parseCommand("group"), { name: "group", arg: "" });
});

test("completion offers command names, then the command's arguments", () => {
  assert.deepEqual(complete("g", cmds, {}), ["goto", "group"]);
  assert.deepEqual(complete("mode h", cmds, { mode: ["ledger", "hunks"] }), ["mode hunks"]);
});

test("a finder hit reads as path:line, then the name it defines or sits in", () => {
  const hit = { i: 3, path: "ppump/geom.py", line: 212, score: 4.2 };
  assert.deepEqual(hitRow({ ...hit, name: "get_blade_wire_between_vertexes" }), [
    "ppump/geom.py:212",
    "get_blade_wire_between_vertexes",
  ]);
  assert.deepEqual(hitRow({ ...hit, name: null }), ["ppump/geom.py:212", ""]);
});
