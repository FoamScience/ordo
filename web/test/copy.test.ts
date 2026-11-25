import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { cutRow, hunkRows, joinPieces, pickedRows, rowsText, toClipboard } from "../src/copy.ts";

const row = (kind: "ctx" | "add" | "edit" | "del", text: string, hunk: number | null = 0) => ({ kind, text, hunk });

test("rows copy as the new side's text: indentation and line breaks kept, nothing else", () => {
  const rows = [row("ctx", "fn main() {"), row("edit", "    let x = 2;"), row("add", "        nested();"), row("ctx", "}")];
  assert.equal(rowsText(rows), "fn main() {\n    let x = 2;\n        nested();\n}");
});

test("an empty line is a line", () => {
  assert.equal(rowsText([row("ctx", "a"), row("ctx", ""), row("ctx", "b")]), "a\n\nb");
});

test("deleted rows are the old side: left out beside live rows, copied alone", () => {
  assert.equal(rowsText([row("ctx", "keep"), row("del", "old"), row("add", "new")]), "keep\nnew");
  assert.equal(rowsText([row("del", "    old one"), row("del", "old two")]), "    old one\nold two");
});

test("the first and last lines of a selection can be partial", () => {
  const rows = [row("ctx", "let alpha = 1;"), row("ctx", "let beta = 2;"), row("ctx", "let gamma = 3;")];
  const text = joinPieces([cutRow(rows[0], 4), cutRow(rows[1]), cutRow(rows[2], 0, 9)]);
  assert.equal(text, "alpha = 1;\nlet beta = 2;\nlet gamma");
});

test("a partial piece of a deleted row alone is the old text", () => {
  assert.equal(joinPieces([cutRow(row("del", "removed line"), 0, 7)]), "removed");
});

test("y takes the picked lines either way round, or the cursor's own line", () => {
  const rows = ["a", "b", "c", "d"];
  assert.deepEqual(pickedRows(rows, 2, 0), ["a", "b", "c"]);
  assert.deepEqual(pickedRows(rows, 1, 3), ["b", "c", "d"]);
  assert.deepEqual(pickedRows(rows, 2, null), ["c"]);
});

test("Y takes the hunk's rows and no other hunk's", () => {
  const rows = [row("ctx", "x", null), row("add", "a", 1), row("del", "b", 1), row("add", "c", 2)];
  assert.equal(rowsText(hunkRows(rows, 1)), "a");
  assert.equal(rowsText(hunkRows([row("del", "gone", 3), row("del", "too", 3)], 3)), "gone\ntoo");
});

test("a clipboard that takes the text is the only one told so", async () => {
  const log: string[] = [];
  await toClipboard(async (t) => void log.push(`wrote ${t}`), "abc", () => log.push("copied"), () => log.push("refused"));
  assert.deepEqual(log, ["wrote abc", "copied"]);
});

test("a clipboard that refuses falls back to the sheet", async () => {
  const log: string[] = [];
  await toClipboard(() => Promise.reject(new Error("denied")), "abc", () => log.push("copied"), () => log.push("refused"));
  assert.deepEqual(log, ["refused"]);
});

test("the gutter, the operator column and the finding marks are not selectable", () => {
  const css = readFileSync(new URL("../src/app.css", import.meta.url), "utf8");
  for (const cls of [".g", ".ln", ".op"]) {
    const rule = css.split("\n").find((l) => l.startsWith(`.l ${cls} {`));
    assert.match(rule ?? "", /user-select: none/, `${cls} must not be selectable`);
  }
});
