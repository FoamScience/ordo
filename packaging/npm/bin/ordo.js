#!/usr/bin/env node
"use strict";
const { spawnSync } = require("child_process");
const { binaryPath } = require("../index.js");
let bin;
try {
  bin = binaryPath("ordo");
} catch (e) {
  console.error(e.message);
  process.exit(1);
}
const res = spawnSync(bin, process.argv.slice(2), { stdio: "inherit" });
if (res.error) {
  console.error(`ordo: ${res.error.message}`);
  process.exit(1);
}
process.exit(res.status ?? 1);
