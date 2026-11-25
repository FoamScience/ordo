// hunch's guess on a hunk, said as a reviewer reads it: how likely it is
// skippable, the features that pushed it there, and whether it moved the
// engine's noise flag; how likely its rationale is faithful; and how much
// attention it needs, which may reorder what the def->use order left free.

import type { Hunch } from "./types.ts";

/** A 0 to 1 share as a whole percent; one rounding everywhere, `–` for a missing or non-finite value. */
export function percent(x: number | undefined): string {
  return x !== undefined && Number.isFinite(x) ? `${Math.round(x * 100)}%` : "–";
}

/** `Hunch: 72% skippable` */
export function badge(g: Hunch): string {
  return `Hunch: ${percent(g.noise)} skippable`;
}

const signed = (top: [string, number][]) =>
  top.map(([name, c]) => `${name} ${c >= 0 ? "+" : "−"}${Math.abs(c).toFixed(2)}`);

/** `comment +1.66`, one per feature, signed toward skipping */
export function contributions(g: Hunch): string[] {
  return signed(g.top);
}

/** `Rationale: 94% faithful`, most moved by features signed toward faithful */
export function faithfulness(g: Hunch): string {
  return `Rationale: ${percent(g.faithful)} faithful, most moved by ${signed(g.faithful_top).join(", ")}`;
}

/** `Attention: 64%`, most moved by features signed toward attention */
export function attention(g: Hunch): string {
  return `Attention: ${percent(g.attention)}, most moved by ${signed(g.attention_top).join(", ")}`;
}

/** `Hunch: shown` / `Hunch: hidden` when the guess moved the hunk's noise flag */
export function flagBadge(g: Hunch): string | null {
  return g.changed === "unhid" ? "Hunch: shown" : g.changed === "hid" ? "Hunch: hidden" : null;
}

/** what the guess did to the review, for the why pane */
export function effect(g: Hunch): string {
  return g.changed === "unhid"
    ? "the engine hid this as noise; the model is confident it should be read, so it is shown"
    : g.changed === "hid"
      ? "the model is confident this can be skipped, so it is hidden like noise"
      : "a trained guess; it hides and shows nothing here";
}
