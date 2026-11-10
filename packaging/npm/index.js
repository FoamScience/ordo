"use strict";
// Thin JS API over the ordo binaries. Each platform's binaries ship in their
// own package (ordo-diff-<platform>-<arch>), installed as an optional
// dependency; ORDO_BIN_DIR overrides it. There is no PATH fallback: the first
// `ordo` on PATH is this package's own shim.
const { spawnSync } = require("child_process");
const path = require("path");

const MAX = 64 * 1024 * 1024;

/** Where binary `name` (`ordo` or `ordo-engine`) lives. */
function binaryPath(name = "ordo-engine") {
  const exe = process.platform === "win32" ? `${name}.exe` : name;
  if (process.env.ORDO_BIN_DIR) return path.join(process.env.ORDO_BIN_DIR, exe);
  const pkg = `ordo-diff-${process.platform}-${process.arch}`;
  try {
    return require.resolve(`${pkg}/bin/${exe}`);
  } catch {
    throw new Error(
      `ordo: ${pkg} is not installed (no prebuilt for this platform, or optional dependencies were skipped); set ORDO_BIN_DIR to a directory holding ${exe}`
    );
  }
}

function runJson(args, input) {
  const bin = binaryPath();
  const res = spawnSync(bin, args, { input, encoding: "utf8", maxBuffer: MAX });
  if (res.error) throw new Error(`ordo: cannot run '${bin}': ${res.error.message}`);
  if (res.status !== 0) throw new Error(`ordo-engine exited ${res.status}: ${res.stderr}`);
  return JSON.parse(res.stdout);
}

/** Order a changeset. `input` matches schema/v2.json's input; returns its output. */
function order(input) {
  return runJson(["order", "--json"], JSON.stringify(input));
}

/** Order a git/unified diff string; returns schema v2 output. */
function review(patch) {
  return runJson(["review"], patch);
}

module.exports = { order, review, binaryPath };
