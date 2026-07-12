"use strict";
// Thin JS API over the ordo binary. Resolves the binary as: ORDO_BIN env →
// vendored (downloaded at install) → `ordo` on PATH.
const { spawnSync } = require("child_process");
const path = require("path");
const fs = require("fs");

const MAX = 64 * 1024 * 1024;

function binaryPath() {
  if (process.env.ORDO_BIN) return process.env.ORDO_BIN;
  const exe = process.platform === "win32" ? "ordo.exe" : "ordo";
  const vendored = path.join(__dirname, "vendor", exe);
  if (fs.existsSync(vendored)) return vendored;
  return exe; // PATH fallback
}

function runJson(args, input) {
  const bin = binaryPath();
  const res = spawnSync(bin, args, { input, encoding: "utf8", maxBuffer: MAX });
  if (res.error) throw new Error(`ordo: cannot run '${bin}': ${res.error.message}`);
  if (res.status !== 0) throw new Error(`ordo exited ${res.status}: ${res.stderr}`);
  return JSON.parse(res.stdout);
}

/** Order a changeset. `input` matches schema/v1 input; returns v1 output. */
function order(input) {
  return runJson(["order", "--json"], JSON.stringify(input));
}

/** Order a git/unified diff string; returns v1 output. */
function review(patch) {
  return runJson(["review"], patch);
}

module.exports = { order, review, binaryPath };
