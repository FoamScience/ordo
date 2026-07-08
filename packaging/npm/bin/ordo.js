#!/usr/bin/env node
"use strict";
// CLI shim: exec the resolved ordo binary with the same args/stdio.
const { spawnSync } = require("child_process");
const { binaryPath } = require("../index.js");
const res = spawnSync(binaryPath(), process.argv.slice(2), { stdio: "inherit" });
process.exit(res.status == null ? 1 : res.status);
