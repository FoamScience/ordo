"use strict";
// postinstall: fetch the platform prebuilt binary from GitHub Releases into
// vendor/. Defensive by design — never hard-fails an install: no-ops when
// ORDO_BIN is set, a binary is already vendored, the platform has no prebuilt,
// or the download fails (dev installs work via ORDO_BIN or `ordo` on PATH).
const fs = require("fs");
const path = require("path");
const https = require("https");

if (process.env.ORDO_BIN || process.env.ORDO_SKIP_DOWNLOAD) process.exit(0);

const VERSION = require("./package.json").version;
const REPO = "elwardi/ordo";
const TARGETS = {
  "linux-x64": "x86_64-unknown-linux-gnu",
  "linux-arm64": "aarch64-unknown-linux-gnu",
  "darwin-x64": "x86_64-apple-darwin",
  "darwin-arm64": "aarch64-apple-darwin",
  "win32-x64": "x86_64-pc-windows-msvc",
};

const key = `${process.platform}-${process.arch}`;
const triple = TARGETS[key];
if (!triple) {
  console.warn(`ordo: no prebuilt for ${key}; set ORDO_BIN or build from source`);
  process.exit(0);
}

const win = process.platform === "win32";
const exe = win ? "ordo.exe" : "ordo";
const dest = path.join(__dirname, "vendor", exe);
if (fs.existsSync(dest)) process.exit(0);

const url = `https://github.com/${REPO}/releases/download/v${VERSION}/ordo-${triple}${win ? ".exe" : ""}`;
fs.mkdirSync(path.dirname(dest), { recursive: true });

function get(u, cb) {
  https
    .get(u, (r) => {
      if (r.statusCode >= 300 && r.statusCode < 400 && r.headers.location) {
        return get(r.headers.location, cb);
      }
      if (r.statusCode !== 200) return cb(new Error(`HTTP ${r.statusCode}`));
      const f = fs.createWriteStream(dest, { mode: 0o755 });
      r.pipe(f);
      f.on("finish", () => f.close(() => cb(null)));
    })
    .on("error", cb);
}

get(url, (err) => {
  if (err) {
    console.warn(`ordo: prebuilt download failed (${err.message}); set ORDO_BIN or build from source`);
  }
});
