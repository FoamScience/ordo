// Bundles src/ into dist/ and stamps dist/source.hash, which a cargo test
// recomputes: a stale bundle fails the Rust build's tests, not a reviewer.
import { build } from "esbuild";
import { writeFileSync, copyFileSync, mkdirSync } from "node:fs";
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
// the faces the page is set in, served from this binary rather than a CDN
mkdirSync("dist/fonts", { recursive: true });
const FONTS = {
  "@fontsource-variable/literata": ["literata-latin-wght-normal.woff2", "literata-latin-wght-italic.woff2"],
  "@fontsource/ibm-plex-mono": [
    "ibm-plex-mono-latin-400-normal.woff2",
    "ibm-plex-mono-latin-600-normal.woff2",
    "ibm-plex-mono-latin-400-italic.woff2",
  ],
};
for (const [pkg, files] of Object.entries(FONTS)) {
  for (const f of files) copyFileSync(`node_modules/${pkg}/files/${f}`, `dist/fonts/${f}`);
  copyFileSync(`node_modules/${pkg}/LICENSE`, `dist/fonts/${pkg.split("/")[1]}.LICENSE`);
}
copyFileSync("src/index.html", "dist/index.html");
writeFileSync("dist/source.hash", sourceHash("src") + "\n");
