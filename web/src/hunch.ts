// hunch's guess on a hunk, said as a reviewer reads it: how likely it is
// skippable, and the features that pushed it there. Shadow only: nothing in
// the review moves on it.

import type { Hunch } from "./types.ts";

/** `Hunch: 72% skippable` */
export function badge(g: Hunch): string {
  return `Hunch: ${Math.round(g.noise * 100)}% skippable`;
}

/** `comment +1.66`, one per feature, signed toward skipping */
export function contributions(g: Hunch): string[] {
  return g.top.map(([name, c]) => `${name} ${c >= 0 ? "+" : "−"}${Math.abs(c).toFixed(2)}`);
}
