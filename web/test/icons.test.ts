import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

// a minimal DOM: enough for the element builder and the icons to run under node
class Txt {
  textContent: string;
  constructor(t: string) {
    this.textContent = t;
  }
}
class El {
  attrs = new Map<string, string>();
  children: (El | Txt)[] = [];
  innerHTML = "";
  tag: string;
  constructor(tag: string) {
    this.tag = tag;
  }
  set className(v: string) {
    this.attrs.set("class", v);
  }
  setAttribute(k: string, v: string) {
    this.attrs.set(k, v);
  }
  getAttribute(k: string) {
    return this.attrs.get(k) ?? null;
  }
  appendChild(c: El | Txt) {
    this.children.push(c);
    return c;
  }
  addEventListener() {}
}
(globalThis as any).document = {
  createElement: (t: string) => new El(t),
  createElementNS: (_: string, t: string) => new El(t),
  createTextNode: (t: string) => new Txt(t),
};

const { ICONS } = await import("../src/icons.ts");
const { fact, heading, pill, said } = await import("../src/chips.ts");
const { changedText, commitsText, coverageText, staleShort, staleWords } = await import("../src/facts.ts");
const { deltas } = await import("../src/readability.ts");
const { percent } = await import("../src/hunch.ts");

/** what a screen reader reads: the aria-label, else the text outside aria-hidden subtrees */
function said_(n: El | Txt): string {
  if (n instanceof Txt) return n.textContent;
  if (n.attrs.get("aria-hidden") === "true") return "";
  return n.attrs.get("aria-label") ?? n.children.map(said_).join("");
}
/** what the eye reads: all text except the screen-reader-only spans */
function seen(n: El | Txt): string {
  if (n instanceof Txt) return n.textContent;
  if (n.attrs.get("class") === "sr" || n.tag === "svg") return "";
  return n.children.map(seen).join("");
}
function find(n: El | Txt, pred: (e: El) => boolean, out: El[] = []): El[] {
  if (n instanceof El) {
    if (pred(n)) out.push(n);
    n.children.forEach((c) => find(c, pred, out));
  }
  return out;
}

test("a heading is an icon, a count and a short word to the eye, and its name and subtitle once to a screen reader", () => {
  const h3 = heading("needs", "needs", "Needs", 3, "what this hunk uses") as unknown as El;
  assert.equal(seen(h3), "3needs");
  assert.equal(said_(h3), "Needs 3: what this hunk uses");
  assert.equal(find(h3, (e) => e.attrs.get("aria-hidden") === "true" && e.attrs.has("title")).length, 1);
  assert.equal(seen(heading("your-note", "note", "Your note") as unknown as El), "note");
  assert.equal(said_(heading("your-note", "note", "Your note") as unknown as El), "Your note");
});

test("a pill or fact shows its short words and tells a screen reader the long ones once", () => {
  const p = pill("hunch", "skip 97%", "Hunch: 97% skippable; most moved by noise +4.47") as unknown as El;
  assert.equal(seen(p), "skip 97%");
  assert.equal(said_(p), "Hunch: 97% skippable; most moved by noise +4.47");
  const f = fact(null, "cx 4→7", "load: cognitive 4 to 7") as unknown as El;
  assert.equal(said_(f), "load: cognitive 4 to 7");
  assert.equal(find(f, (e) => e.tag === "svg").length, 0);
  for (const e of [p, f]) {
    assert.equal(find(e, (x) => x.attrs.get("class") === "sr").length, 1);
    assert.equal(e.attrs.has("title"), false, "the tooltip sits on the hidden glance, not on the host");
  }
});

test("a button's accessible name can begin with what it shows", () => {
  const span = said([new El("b") as unknown as Node], "changed 3 since your last look", false);
  assert.equal(span.length, 2);
  assert.equal(span[0].getAttribute("title"), null);
  assert.equal(said_(span[1] as unknown as El), "changed 3 since your last look");
});

test("every icon the sources name exists and every icon is used", () => {
  const src = ["main.ts", "chips.ts"].map((f) => readFileSync(new URL(`../src/${f}`, import.meta.url), "utf8")).join("\n");
  for (const name of Object.keys(ICONS)) assert.ok(src.includes(`"${name}"`), `${name} is never used`);
  for (const [name, i] of Object.entries(ICONS)) assert.match(i.body, /<(path|circle|line|rect|polyline|polygon|ellipse)\b/, `${name} draws nothing`);
});

test("one rounding for a share, and a dash for a missing one", () => {
  assert.equal(percent(0.715), "72%");
  assert.equal(percent(0.7149), "71%");
  assert.equal(percent(0), "0%");
  assert.equal(percent(Number.NaN), "–");
  assert.equal(percent(undefined), "–");
});

test("the stale line shrinks to a number, and its words begin with what the band shows", () => {
  assert.equal(staleShort("stale · 2 files changed · r reloads"), "2");
  assert.equal(staleShort("stale · committed, 1 file changed · r reloads, :e main...zz keeps the commits in view"), "1");
  assert.equal(staleShort("stale · wave 3 recorded · r reloads"), "w3");
  assert.equal(staleShort("stale · HEAD moved (rebase or branch switch) · r reloads"), "HEAD");
  assert.deepEqual(staleWords("stale · 2 files changed · r reloads"), { short: "stale 2", words: "stale 2, 2 files changed · r reloads" });
});

test("short facts", () => {
  assert.equal(commitsText(1), "1 commit in 6 months");
  assert.equal(commitsText(8), "8 commits in 6 months");
  assert.equal(changedText(1, false), "changed 1 time");
  assert.equal(changedText(0, false), "changed 0 times");
  assert.equal(changedText(5, true), "changed 5+ times");
  assert.equal(coverageText([12, 14]), "tests 12/14 lines");
  assert.equal(coverageText([Number.NaN, 14]), "tests –");
});

test("readability keeps every moved measure as before to after, with its words", () => {
  const d = { name: "f", nesting: [2, 5], lines: [10, 9], params: [1, 1], branches: [3, 3], cognitive: [4, 7], short_names: [10, 25] } as Parameters<typeof deltas>[0];
  assert.deepEqual(
    deltas(d).map((x) => [x.text, x.words]),
    [
      ["nest 2→5", "nesting 2 to 5"],
      ["cx 4→7", "cognitive 4 to 7"],
      ["short 10→25%", "short names 10% to 25%"],
      ["lines 10→9", "lines 10 to 9"],
    ],
  );
});
