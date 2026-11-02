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
// the stylesheet's @imports inlined; the fonts stay files beside it
await build({
  entryPoints: ["src/app.css"],
  bundle: true,
  outfile: "dist/app.css",
  external: ["fonts/*"],
  legalComments: "none",
});
// the faces the page is set in, served from this binary rather than a CDN
mkdirSync("dist/fonts", { recursive: true });
const FONTS = {
  "@fontsource-variable/geist-mono": ["geist-mono-latin-wght-normal.woff2", "geist-mono-latin-wght-italic.woff2"],
};
for (const [pkg, files] of Object.entries(FONTS)) {
  for (const f of files) copyFileSync(`node_modules/${pkg}/files/${f}`, `dist/fonts/${f}`);
  copyFileSync(`node_modules/${pkg}/LICENSE`, `dist/fonts/${pkg.split("/")[1]}.LICENSE`);
}
copyFileSync("src/index.html", "dist/index.html");
writeFileSync("dist/source.hash", sourceHash("src") + "\n");
