// The glance-and-words markup of the margin, the band and the pills: an icon
// and a short value for the eye, the words once for a screen reader.

import { h } from "./dom.ts";
import { icon, type IconName } from "./icons.ts";

type Child = Node | string | number | null | undefined | false | Child[];

/** The glance hidden from assistive technology, carrying the tooltip, then the words, said once. `tip` false leaves the tooltip to the host (a button's own title). */
export function said(glance: Child[], words: string, tip: string | false = words): HTMLElement[] {
  return [h("span", { class: "tip", "aria-hidden": "true", title: tip || null }, glance), h("span", { class: "sr" }, words)];
}

/** An icon and its shortest value with its word (`skip 97%`, `wave 2`). */
export function pill(name: IconName, value: string, words: string, cls = ""): HTMLElement {
  return h("span", { class: `pill${cls && ` ${cls}`}` }, said([icon(name), h("span", { class: "v" }, value)], words));
}

/** An icon and a fact with its word, in a margin note. */
export function fact(name: IconName | null, value: string, words: string, cls = ""): HTMLElement {
  return h("span", { class: `fact${cls && ` ${cls}`}` }, said([name && icon(name), h("span", { class: "v" }, value)], words));
}

/** A margin heading: an icon, a count and a short word to the eye; the title and subtitle are the tooltip and what a screen reader hears. */
export function heading(name: IconName, word: string, title: string, count?: number | string, hint = ""): HTMLElement {
  const words = `${title}${count === undefined ? "" : ` ${count}`}${hint ? `: ${hint}` : ""}`;
  return h(
    "h3",
    null,
    said([icon(name), count !== undefined && h("span", { class: "v" }, count), h("span", { class: "w" }, word)], words, hint ? `${title}: ${hint}` : title),
  );
}
