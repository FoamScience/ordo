"use strict";
// Lays out the npm packages for one release: ordo-diff itself, depending on
// one ordo-diff-<platform>-<arch> package per prebuilt target, each holding
// that target's binaries.
//
//   node stage.js <version> <bins> <out>
//
// <bins>/bin-<target>/ holds the release build of `ordo` and `ordo-engine`
// for <target>; <out> receives one directory per package, ready for
// `npm publish`.
const fs = require("fs");
const path = require("path");

const TARGETS = {
  "linux-x64": "x86_64-unknown-linux-musl",
  "linux-arm64": "aarch64-unknown-linux-musl",
  "darwin-x64": "x86_64-apple-darwin",
  "darwin-arm64": "aarch64-apple-darwin",
  "win32-x64": "x86_64-pc-windows-msvc",
};

const [version, bins, out] = process.argv.slice(2);
if (!version || !bins || !out) {
  console.error("usage: node stage.js <version> <bins> <out>");
  process.exit(2);
}

const main = JSON.parse(fs.readFileSync(path.join(__dirname, "package.json"), "utf8"));
main.version = version;
main.optionalDependencies = {};

for (const [platform, target] of Object.entries(TARGETS)) {
  const [os, cpu] = platform.split("-");
  const name = `${main.name}-${platform}`;
  const dir = path.join(out, name);
  fs.mkdirSync(path.join(dir, "bin"), { recursive: true });
  const exe = os === "win32" ? ".exe" : "";
  for (const bin of ["ordo", "ordo-engine"]) {
    const dest = path.join(dir, "bin", bin + exe);
    fs.copyFileSync(path.join(bins, `bin-${target}`, bin + exe), dest);
    fs.chmodSync(dest, 0o755);
  }
  const pkg = {
    name,
    version,
    description: `${main.name} binaries for ${platform}`,
    license: main.license,
    repository: main.repository,
    os: [os],
    cpu: [cpu],
    files: ["bin/"],
  };
  fs.writeFileSync(path.join(dir, "package.json"), JSON.stringify(pkg, null, 2) + "\n");
  main.optionalDependencies[name] = version;
}

const dir = path.join(out, main.name);
fs.mkdirSync(path.join(dir, "bin"), { recursive: true });
for (const f of ["index.js", "README.md", "bin/ordo.js", "bin/ordo-engine.js"]) {
  fs.copyFileSync(path.join(__dirname, f), path.join(dir, f));
}
fs.copyFileSync(path.join(__dirname, "..", "..", "LICENSE"), path.join(dir, "LICENSE"));
fs.writeFileSync(path.join(dir, "package.json"), JSON.stringify(main, null, 2) + "\n");
