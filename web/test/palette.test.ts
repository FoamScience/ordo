import { test } from "node:test";
import assert from "node:assert/strict";
import { complete, parseCommand } from "../src/palette.ts";

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
