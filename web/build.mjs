// Bundles src/ into dist/ and stamps dist/source.hash, which a cargo test
// recomputes: a stale bundle fails the Rust build's tests, not a reviewer.
import { build } from "esbuild";
import { readFileSync, readdirSync, writeFileSync, copyFileSync } from "node:fs";
import { join } from "node:path";
import { sourceHash } from "./hash.mjs";

await build({
  entryPoints: ["src/main.ts"],
  bundle: true,
  format: "esm",
  target: "es2022",
  outfile: "dist/app.js",
  legalComments: "none",
});
copyFileSync("src/app.css", "dist/app.css");
copyFileSync("src/index.html", "dist/index.html");
writeFileSync("dist/source.hash", sourceHash("src") + "\n");
