import { test } from "node:test";
import assert from "node:assert/strict";
import { dismissToggle } from "../src/dismiss.ts";
import type { Finding } from "../src/types.ts";

const f: Finding = { source: "catalog", name: "goto", message: "m", level: "note" };

test("a catalog finding can be dismissed, and dismissed again to undo it", () => {
  assert.deepEqual(
    [dismissToggle(f, [])?.label, dismissToggle(f, ["goto"])?.label, dismissToggle(f, ["magic-number"])?.label],
    ["dismiss", "dismissed", "dismiss"],
  );
  assert.equal(dismissToggle(f, [])?.command, "dismiss goto");
});

test("only catalog findings offer it", () => {
  for (const source of ["rule", "analyzer", "review"]) assert.equal(dismissToggle({ ...f, source }, []), null);
});
