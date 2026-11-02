// FNV-1a 64 over every file under a directory, sorted by path: each file
// contributes its relative path, a NUL, its bytes and a NUL. tests/web_dist.rs
// computes the same value.
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";

function files(dir) {
  return readdirSync(dir).flatMap((name) => {
    const p = join(dir, name);
    return statSync(p).isDirectory() ? files(p) : [p];
  });
}

export function sourceHash(dir) {
  let h = 0xcbf29ce484222325n;
  const feed = (bytes) => {
    for (const b of bytes) {
      h ^= BigInt(b);
      h = (h * 0x100000001b3n) & 0xffffffffffffffffn;
    }
  };
  for (const p of files(dir).map((p) => relative(dir, p).split("\\").join("/")).sort()) {
    feed(new TextEncoder().encode(p));
    feed([0]);
    feed(readFileSync(join(dir, p)));
    feed([0]);
  }
  return h.toString(16).padStart(16, "0");
}
