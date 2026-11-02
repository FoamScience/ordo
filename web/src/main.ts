// The browser half of `ordo web`. The server owns the review — what each hunk
// means, what is reviewed, what a command does — and this file lays it out and
// turns keys and clicks into requests.

import { Api } from "./api.ts";
import { $, h, replace } from "./dom.ts";
import { Keymap, keyLabel } from "./keys.ts";
import { centerScroll, codeRows, firstRowOf, runs, stripMarks, tableLimit, wordAt, wordMove, type CodeRow } from "./layout.ts";
import { rowText, withSymbols } from "./list.ts";
import { cascadeOf, depMap, groundOf, needPairs, smoothPath } from "./depmap.ts";
import { CLIENT_COMMANDS, complete, parseCommand } from "./palette.ts";
import { files as fileLines, flagged, written } from "./summary.ts";
import { applyTheme, loadThemes, themeNames } from "./theme.ts";
import type { ActResult, FileData, Item, ItemFacts, Level, Review, Row } from "./types.ts";

type Pane = "list" | "code" | "why";
const PANES: Pane[] = ["list", "code", "why"];
const LEVELS: Level[] = ["verdict", "warn", "note"];
const SINCE: Record<string, [string, string]> = {
  new: ["new", "new since you last looked"],
  changed: ["changed", "changed since you last looked"],
  moved: ["moved", "unchanged, but it reads in a different place now: its dependencies moved"],
};
const CHORD_TIMEOUT = 500;
/** The gutter mark for each kind of code row; `~` is a line that only gained text. */
const OP: Record<CodeRow["kind"], string> = { ctx: " ", add: "+", del: "-", edit: "~" };

const rev = new URLSearchParams(location.search).get("rev") ?? "HEAD";
const token =
  new URLSearchParams(location.hash.slice(1)).get("t") ?? sessionStorage.getItem("ordo.token") ?? "";
sessionStorage.setItem("ordo.token", token);
// the token stays in this tab's storage, not in its history or bookmarks
if (location.hash) history.replaceState(null, "", location.pathname + location.search);
// a fresh link pasted into this tab changes only the hash, which reloads nothing
window.addEventListener("hashchange", () => {
  const fresh = new URLSearchParams(location.hash.slice(1)).get("t");
  if (!fresh) return;
  sessionStorage.setItem("ordo.token", fresh);
  history.replaceState(null, "", location.pathname + location.search);
  location.reload();
});
const api = new Api(token, rev);

interface Search {
  query: string;
  hits: number[];
  k: number;
}

const st = {
  review: null as Review | null,
  status: "starting…",
  version: -1,
  keymap: new Keymap([]),
  sel: -1,
  focus: "list" as Pane,
  zoom: false,
  file: null as FileData | null,
  rows: [] as CodeRow[],
  files: new Map<string, FileData>(),
  facts: null as ItemFacts | null,
  cursor: 0,
  from: null as number | null,
  jumps: [] as { sel: number; cursor: number }[],
  pending: null as string | null,
  pendingTimer: 0,
  search: null as Search | null,
  sinceOnly: false,
  empty: false,
  col: null as number | null,
  whyLinks: [] as number[],
  whyCur: -1,
  notice: "",
  /** the one-page summary stands in for the columns */
  summary: false,
};

// ------------------------------------------------------------------ data

// requests that overlap answer in any order: only the latest one lands
let refreshSeq = 0;
let selectSeq = 0;
let filesGen = 0;

async function refresh(): Promise<void> {
  const seq = ++refreshSeq;
  let slot;
  try {
    slot = await api.review();
  } catch (e) {
    if (seq !== refreshSeq) return;
    st.status = String((e as Error).message);
    return renderAll();
  }
  if (seq !== refreshSeq) return;
  if (slot.state !== "ready") {
    st.status = slot.status;
    st.empty = slot.state === "empty";
    st.review = null;
    return renderAll();
  }
  st.empty = false;
  if (slot.version < st.version && slot.generation === st.review?.generation) return;
  const prev = st.review;
  st.review = slot;
  st.version = slot.version;
  st.keymap = new Keymap(slot.keys);
  // a reload renumbers the hunks: find each one held on to by what it is
  const moved = !!prev && prev.generation !== slot.generation;
  const remap = (i: number): number => {
    if (!prev || !moved) return i;
    const id = identity(prev.items[i]);
    return id ? slot.items.findIndex((it) => identity(it) === id) : -1;
  };
  let sel = st.sel >= 0 ? remap(st.sel) : -1;
  if (sel < 0) sel = prev ? clampToView(Math.min(st.sel, slot.items.length - 1)) : slot.sel;
  if (moved) {
    st.jumps = st.jumps.map((j) => ({ ...j, sel: remap(j.sel) })).filter((j) => j.sel >= 0);
    if (depmap) {
      depmap.root = remap(depmap.root);
      depmap.sel = remap(depmap.sel);
      if (depmap.root < 0) closeSheet();
    }
    if (canvas) {
      const c = canvas;
      c.anchor = remap(c.anchor);
      c.cards = c.cards.map((k) => ({ ...k, i: remap(k.i) })).filter((k) => k.i >= 0);
      if (c.anchor < 0 || !c.cards.length) {
        canvas = null;
        closeSheet();
      } else {
        c.sel = Math.min(c.sel, c.cards.length - 1);
      }
    }
  }
  if (slot.delta.new + slot.delta.changed + slot.delta.moved === 0) st.sinceOnly = false;
  if (prev?.version !== slot.version) {
    st.files.clear();
    filesGen++;
    st.facts = null;
  }
  const same = sel === st.sel && !!st.file;
  if (same) st.sel = -1;
  await select(sel, { keepCursor: same });
  if (canvas) renderCanvas();
  if (depmap) renderMap(true);
}

function identity(it: Item | undefined): string {
  return it ? `${it.path}\u0000${it.new[0]}\u0000${it.rationale}` : "";
}

function clampToView(i: number): number {
  const v = st.review!.view;
  return v.includes(i) ? i : (v[0] ?? 0);
}

async function fileOf(path: string): Promise<FileData> {
  const hit = st.files.get(path);
  if (hit) return hit;
  const gen = filesGen;
  const f = await api.file(path);
  if (gen === filesGen) st.files.set(path, f);
  return f;
}

let factsFor = -1;
async function loadFacts(i: number): Promise<void> {
  factsFor = i;
  const version = st.version;
  try {
    const f = await api.item(i);
    if (factsFor === i && version === st.version) {
      st.facts = f;
      renderWhy();
      renderBand();
    }
  } catch {
    // the review reloaded underneath; the refresh that follows asks again
  }
}

/**
 * Makes hunk `i` the one being read. `keepCursor` is for a cursor already
 * standing in `i`'s file — a line move or a click that crossed into it.
 */
async function select(i: number, opts: { keepCursor?: boolean; focus?: Pane } = {}): Promise<void> {
  const r = st.review;
  if (!r || i < 0 || i >= r.items.length) return renderAll();
  const seq = ++selectSeq;
  const it = r.items[i];
  let f: FileData;
  try {
    f = await fileOf(it.path);
  } catch (e) {
    if (seq === selectSeq) toast((e as Error).message, true);
    return;
  }
  if (seq !== selectSeq || st.review !== r) return;
  const changed = i !== st.sel;
  if (changed && st.file?.path !== it.path) st.search = null;
  st.sel = i;
  if (opts.focus) st.focus = opts.focus;
  st.file = f.missing ? null : f;
  st.rows = st.file ? codeRows(st.file) : [];
  if (!opts.keepCursor) {
    st.col = null;
    st.cursor = firstRowOf(st.rows, i);
    st.from = null;
  }
  st.cursor = Math.min(st.cursor, Math.max(0, st.rows.length - 1));
  if (changed) {
    st.facts = null;
    st.whyCur = -1;
  }
  renderAll();
  scrollCodeTo(firstRowOf(st.rows, i), !opts.keepCursor || changed);
  loadFacts(i);
}

async function act(body: Record<string, unknown>): Promise<ActResult | null> {
  try {
    const out = await api.act({
      sel: st.sel,
      generation: st.review?.generation,
      line: lineOfCursor(),
      from: fromLine(),
      ...body,
    });
    if (out.error) toast(out.error, true);
    if (out.notice) toast(out.notice);
    if (out.popup) showSheet(out.popup.title, out.popup.lines.join("\n"));
    if (out.clipboard !== undefined) {
      await navigator.clipboard.writeText(out.clipboard).then(
        () => toast("copied the review prompt to the clipboard"),
        () => showSheet("review prompt", out.clipboard!),
      );
    }
    if (out.navigate) {
      const q = new URLSearchParams({ rev: out.navigate });
      location.href = `/?${q}#t=${token}`;
    }
    if (out.version > st.version) await refresh();
    // a command can move the selection (`:goto`, `]f`, a filter that hid it)
    if (out.sel !== undefined && out.sel !== st.sel) await select(out.sel);
    renderConfig();
    return out;
  } catch (e) {
    toast((e as Error).message, true);
    // a 409 is a review read again underneath: catch up with it
    refresh();
    return null;
  }
}

/** The new-side line (0-based) the code cursor stands on, as the server's cursor. */
function lineOfCursor(): number | undefined {
  const row = st.rows[st.cursor];
  if (!row) return undefined;
  if (row.kind !== "del") return row.n - 1;
  const next = st.rows.slice(st.cursor).find((r): r is Exclude<CodeRow, { kind: "del" }> => r.kind !== "del");
  return next ? next.n - 1 : undefined;
}

function fromLine(): number | undefined {
  if (st.from === null) return undefined;
  const row = st.rows[st.from];
  return row && row.kind !== "del" ? row.n - 1 : undefined;
}

// ------------------------------------------------------------------ list

/** The list's rows as shown: with `since` on, only hunks that changed since the last look. */
function shownRows(): Row[] {
  const r = st.review!;
  if (!st.sinceOnly) return r.rows;
  const keep = (i: number) => r.items[i].delta !== null;
  const out: Row[] = [];
  for (const row of r.rows) {
    if (row.kind === "item" && !keep(row.i)) continue;
    if (row.kind === "folded" && !row.items.some(keep)) continue;
    if (row.kind === "gap") continue;
    if (row.kind === "file" && out.at(-1)?.kind === "file") out.pop();
    out.push(row);
  }
  if (out.at(-1)?.kind === "file") out.pop();
  return out;
}

/** Where `j`/`k` stop: a hunk, or a fold standing for the hunks it hides. */
function stops(): number[] {
  return shownRows().flatMap((row) =>
    row.kind === "item" ? [row.i] : row.kind === "folded" ? [row.items.includes(st.sel) ? st.sel : row.items[0]] : [],
  );
}

function step(by: number): void {
  const s = stops();
  if (!s.length) return;
  const at = s.indexOf(st.sel);
  const to = at < 0 ? 0 : Math.max(0, Math.min(s.length - 1, at + by));
  select(s[to]);
}

function rowClass(items: number[], selected: boolean): string {
  const r = st.review!;
  const its = items.map((i) => r.items[i]);
  const reviewed = its.every((it) => it.reviewed);
  const level = LEVELS.find((l) => its.some((it) => it.level === l));
  const quiet = reviewed || its.every((it) => it.noise);
  return ["r", reviewed && "reviewed", level && `lv-${level}`, selected && "sel", quiet && "quiet"].filter(Boolean).join(" ");
}

function renderList(): void {
  const pane = $("#list");
  const r = st.review!;
  const scope = [
    `${r.view.length} hunk${r.view.length === 1 ? "" : "s"}`,
    r.mode === "ledger" && "by symbol",
    st.sinceOnly && "changed since your last look",
    r.path_filter,
  ].filter(Boolean);
  replace(
    pane.querySelector(".head")!,
    h("span", { class: "t" }, "Reading order"),
    h("span", { class: "sub" }, scope.join(", ")),
    h("span", { class: "grow" }),
    zoomButton("list"),
  );
  const body = pane.querySelector(".body")!;
  const rows = h("div", { class: "rows" });
  const dots = new Map<number, HTMLElement>();
  let selEl: HTMLElement | null = null;
  const { rows: listRows, runOf } = withSymbols(shownRows(), r.items);
  for (const row of listRows) {
    if (row.kind === "symbol") {
      rows.append(
        h(
          "div",
          {
            class: `symhead${row.items.includes(st.sel) ? " cur" : ""}`,
            title: `${row.items.length} hunks in a row change ${row.name}`,
            onclick: () => {
              st.focus = "list";
              select(row.items.find((i) => !r.items[i].reviewed) ?? row.items[0]);
            },
          },
          h("span", { class: "code-word" }, row.name),
          h("span", null, `${row.items.length} hunks`),
        ),
      );
    } else if (row.kind === "file") {
      const cut = row.path.lastIndexOf("/") + 1;
      rows.append(
        h(
          "div",
          { class: "fileband", title: row.path },
          h("span", { class: "path" }, h("span", { class: "dir" }, row.path.slice(0, cut)), h("span", { class: "base" }, row.path.slice(cut))),
          h("span", null, row.n),
        ),
      );
    } else if (row.kind === "gap") {
      rows.append(h("div", { class: "gap" }));
    } else {
      const items = row.kind === "item" ? [row.i] : row.items;
      const first = r.items[items[0]];
      const selected = items.includes(st.sel);
      const folded = row.kind === "folded";
      const level = LEVELS.find((l) => items.some((i) => r.items[i].level === l));
      const run = folded ? undefined : runOf.get(items[0]);
      const said = rowText(first, run ? run.map((i) => r.items[i]) : null);
      const removal = first.new[1] < first.new[0];
      const el = h(
        "div",
        {
          class: `${rowClass(items, selected)}${folded ? " folded" : ""}${run ? " inrun" : ""}`,
          title: folded ? first.symbol ?? "" : first.rationale,
          onclick: () => {
            st.focus = "list";
            select(items.includes(st.sel) ? st.sel : items[0]);
          },
        },
        h(
          "span",
          { class: "txt" },
          run && h("span", { class: "ln" }, `L${removal ? first.old[0] : first.new[0]}`),
          folded
            ? `${items.length} hunk${items.length === 1 ? "" : "s"}, folded`
            : said.code
              ? h("span", { class: `gist${removal ? " removed" : ""}` }, said.text)
              : sentence(said.text),
        ),
        h(
          "span",
          { class: "meta" },
          first.delta && h("span", { class: "since", title: SINCE[first.delta][1] }, SINCE[first.delta][0]),
          level && h("span", { class: `lv lv-${level}`, title: `${level} finding` }, level === "warn" ? "warning" : level),
        ),
      );
      for (const i of items) dots.set(i, el);
      if (selected) selEl = el;
      rows.append(el);
    }
  }
  replace(body, rows);
  drawArcs(rows, dots);
  if (selEl) {
    const el = selEl as HTMLElement;
    body.scrollTop = centerScroll(el.offsetTop, el.offsetHeight, body.clientHeight, rows.offsetHeight);
  }
}

/**
 * The selected hunk's dependencies, drawn as arcs off the reading thread:
 * solid to what it needs, dashed to what needs it.
 */
function drawArcs(rows: HTMLElement, dots: Map<number, HTMLElement>): void {
  const it = st.review!.items[st.sel];
  const from = dots.get(st.sel);
  if (!it || !from) return;
  const rail = parseFloat(getComputedStyle(rows).getPropertyValue("--rail")) || 30;
  const mid = (el: HTMLElement) => el.offsetTop + el.offsetHeight / 2;
  const y0 = mid(from);
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("class", "arcs");
  svg.setAttribute("height", String(rows.offsetHeight));
  svg.setAttribute("aria-hidden", "true");
  const arcs = new Map<HTMLElement, boolean>();
  for (const e of it.edges) {
    const to = e.target === null ? undefined : dots.get(e.target);
    if (to && to !== from) arcs.set(to, (arcs.get(to) ?? false) || e.needs);
  }
  // nearer arcs run inside farther ones, each in a lane of its own
  const lanes = [...arcs].sort(([a], [b]) => Math.abs(mid(a) - y0) - Math.abs(mid(b) - y0));
  const step = Math.max(2, (rail - 10) / Math.max(1, lanes.length));
  lanes.forEach(([to, needs], k) => {
    const y1 = mid(to);
    const bulge = Math.min(rail - 4, 7 + k * step);
    const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
    path.setAttribute("d", `M ${rail} ${y0} C ${rail - bulge * 1.3} ${y0}, ${rail - bulge * 1.3} ${y1}, ${rail} ${y1}`);
    path.setAttribute("class", needs ? "needs" : "used");
    svg.append(path);
  });
  rows.prepend(svg);
}

// ------------------------------------------------------------------ code

function rowHeight(): number {
  return document.querySelector<HTMLElement>("#code .l")?.offsetHeight || 26;
}

/** Puts `row` three rows below the top of the code pane. */
function scrollCodeTo(row: number, force: boolean): void {
  if (!force) return;
  const body = $("#code .body");
  const el = body.querySelector<HTMLElement>(`.l[data-k="${Math.max(0, row - 3)}"]`);
  body.scrollTop = el ? el.offsetTop : 0;
}

function followCursor(): void {
  const body = $("#code .body");
  const rh = rowHeight();
  const el = body.querySelector<HTMLElement>(".l.cur");
  const top = el ? el.offsetTop : st.cursor * rh;
  if (top < body.scrollTop) body.scrollTop = top;
  else if (top + rh > body.scrollTop + body.clientHeight) body.scrollTop = top + rh - body.clientHeight;
}

function lineEl(row: CodeRow, k: number, mine: boolean, findings: Map<number, Item["findings"]>, uses: Set<number>, commented: Set<number>): HTMLElement[] {
  const search = st.search;
  const caret = k === st.cursor ? caretCol(row.text) : -1;
  let at = 0;
  const parts = runs(row.text, row.spans, row.marks, row.kind === "edit" ? row.gone : []).flatMap((run) => {
    // what the old line had here: shown, but no column of this one
    if (run.gone) return [h("del", { class: "gone", title: "removed" }, run.text)];
    const piece = (text: string, extra?: string) =>
      h(run.changed ? "mark" : "span", { class: [run.role && `s-${run.role}`, extra].filter(Boolean).join(" ") || null }, text);
    const cs = [...run.text];
    const i = caret - at;
    at += cs.length;
    if (i < 0 || i >= cs.length) return [piece(run.text)];
    const before = cs.slice(0, i).join("");
    const after = cs.slice(i + 1).join("");
    return [before && piece(before), piece(cs[i], "caret"), after && piece(after)].filter((x): x is HTMLElement => !!x);
  });
  const content = search && search.hits.includes(k) ? highlightHits(row.text, search, k) : parts;
  const n = row.kind === "del" ? "" : row.n;
  const cls = [
    "l",
    row.kind,
    row.hunk !== null && !mine && "other",
    k === st.cursor && "cur",
    st.from !== null && k >= Math.min(st.from, st.cursor) && k <= Math.max(st.from, st.cursor) && "picked",
    row.kind !== "del" && commented.has(row.n) && "commented",
  ]
    .filter(Boolean)
    .join(" ");
  const out: HTMLElement[] = [
    h(
      "div",
      {
        class: cls,
        dataset: { k: String(k) },
        onclick: (e: MouseEvent) => {
          st.cursor = k;
          st.col = caretColumn(e, row.text);
          st.focus = "code";
          if (row.hunk !== null && row.hunk !== st.sel) select(row.hunk, { keepCursor: true, focus: "code" });
          else renderCode();
        },
        onmousemove: (e: MouseEvent) => row.kind !== "del" && peekLater(e, row.n, row.text),
        onmouseleave: () => peekLeave(),
      },
      h("span", { class: "g" }, row.kind !== "del" && uses.has(row.n) ? "›" : ""),
      h("span", { class: "ln" }, n),
      h("span", { class: "op" }, OP[row.kind]),
      h("span", null, content),
    ),
  ];
  if (row.kind !== "del") {
    for (const f of findings.get(row.n) ?? []) {
      out.push(h("div", { class: `fl ${f.level}` }, h("b", null, `${f.level} ${f.name}`), f.message.split("\n")[0]));
    }
  }
  return out;
}

function highlightHits(text: string, s: Search, k: number): Node[] {
  const out: Node[] = [];
  const low = text.toLowerCase();
  const q = s.query.toLowerCase();
  let at = 0;
  for (let p = low.indexOf(q); q && p >= 0; p = low.indexOf(q, p + q.length)) {
    out.push(document.createTextNode(text.slice(at, p)));
    out.push(h("span", { class: `hit${s.hits[s.k] === k ? " now" : ""}` }, text.slice(p, p + q.length)));
    at = p + q.length;
  }
  out.push(document.createTextNode(text.slice(at)));
  return out;
}

/** The reading column's words: where the hunk is, what it does, and what the reader can do next. */
function renderReading(it: Item | undefined): void {
  const r = st.review!;
  const head = $("#code > .head");
  const actions = $("#code .foot .actions");
  if (!it) {
    replace(head);
    replace(actions);
    return;
  }
  const pos = r.view.indexOf(st.sel);
  const what = it.what.length ? it.what : ["a change ordo found nothing to say about"];
  const search = st.search;
  replace(
    head,
    h(
      "div",
      { class: "kicker" },
      h("span", null, pos >= 0 ? `Hunk ${pos + 1} of ${r.view.length}` : "Outside the current filter"),
      h("span", { class: "path" }, `${it.path}:${it.new[0]}`),
      h("span", { class: "grow" }),
      zoomButton("code"),
    ),
    h("h1", { class: "headline" }, prose(sentence(what[0]))),
    (what.length > 1 || it.details.length > 0 || it.grouped_by) &&
      h(
        "div",
        { class: "lede" },
        what.slice(1).map((w) => h("p", null, prose(sentence(w)))),
        it.details.map((d) => h("p", { class: "detail" }, prose(sentence(d)))),
        it.grouped_by && h("p", null, "Grouped by ", prose(it.grouped_by)),
      ),
    h(
      "div",
      { class: "pills" },
      it.reviewed && h("span", { class: "pill done" }, "Read"),
      it.delta && h("span", { class: "pill accent" }, sentence(SINCE[it.delta][1].split(":")[0])),
      it.noise && !/formatting/.test(it.rationale) && h("span", { class: "pill" }, "Quiet: generated or formatting"),
      it.wave !== null && h("span", { class: "pill" }, `Wave ${it.wave}`),
    ),
    (search || st.from !== null) &&
      h(
        "div",
        { class: "status" },
        search && h("span", null, `Searching for “${search.query}”: ${search.hits.length ? `${search.k + 1} of ${search.hits.length}` : "no match"}`),
        st.from !== null && h("span", null, "Selecting lines; press c to comment on them"),
      ),
  );
  const key = st.keymap.keysFor("toggle-reviewed")[0];
  const s = stops();
  const at = s.indexOf(st.sel);
  const next = at >= 0 && at + 1 < s.length ? r.items[s[at + 1]] : null;
  replace(
    actions,
    h(
      "button",
      { class: `read${it.reviewed ? " done" : ""}`, onclick: () => run("toggle-reviewed"), title: it.reviewed ? "Mark as unread" : "Mark as read and go to the next hunk" },
      it.reviewed ? "Marked as read" : "Mark as read",
      key && h("kbd", null, key),
    ),
    next && h("button", { class: "next", onclick: () => step(1), title: `Next: ${next.rationale}`, "aria-label": `next hunk: ${next.rationale}` }, "Next"),
    next && h("span", { class: "said" }, prose(sentence(next.rationale))),
  );
}

/** ordo's sentences start lower-case, to read as a list; a page wants capitals. */
function sentence(s: string): string {
  return /^[a-z]/.test(s) && !NAME_AT_START.test(s) ? s[0].toUpperCase() + s.slice(1) : s;
}

// a name in a sentence: dotted, snake_case, camelCase, a path, a call
const NAME = /`[^`]+`|[A-Za-z_][\w]*(?:[._/:][\w]+)+|\b[a-z]+[A-Z]\w*|\b[A-Z][a-z0-9]+[A-Z]\w*|\b\w+\(\)/g;
const NAME_AT_START = /^(?:[A-Za-z_][\w]*(?:[._/:][\w]+)+|[a-z]+[A-Z]\w*)/;

/** A sentence with the names in it set as code. */
function prose(text: string): (string | HTMLElement)[] {
  const out: (string | HTMLElement)[] = [];
  let at = 0;
  for (const m of text.matchAll(NAME)) {
    if (m.index! > at) out.push(text.slice(at, m.index));
    out.push(h("span", { class: "code-word" }, m[0].replace(/^`|`$/g, "")));
    at = m.index! + m[0].length;
  }
  if (at < text.length) out.push(text.slice(at));
  return out;
}

function renderCode(): void {
  const pane = $("#code");
  const r = st.review!;
  const it = r.items[st.sel];
  renderReading(it);
  const body = pane.querySelector(".body")!;
  if (!st.file || !it) return replace(body);
  const findings = new Map<number, Item["findings"]>();
  for (const f of it.findings) {
    if (f.row == null) continue;
    findings.set(f.row, [...(findings.get(f.row) ?? []), f]);
  }
  const uses = new Set(it.uses_at.flatMap((u) => u.rows));
  const commented = new Set(st.file.comments.flatMap((c) => range(c.start, c.end)));
  const code = h("div", { class: "code" });
  st.rows.forEach((row, k) => {
    for (const el of lineEl(row, k, row.hunk === st.sel, findings, uses, commented)) code.append(el);
    if (row.kind !== "del") {
      for (const c of st.file!.comments.filter((c) => c.end === row.n)) {
        code.append(h("div", { class: "cmt" }, `${c.text}${c.stale ? " (lines changed since)" : ""}`));
      }
    }
  });
  const top = body.scrollTop;
  replace(body, code);
  body.scrollTop = top;
  renderStrip();
}

// ------------------------------------------------------------------ strip beside the code

function renderStrip(): void {
  const pane = $("#code");
  const body = pane.querySelector<HTMLElement>(".body")!;
  let strip = pane.querySelector<HTMLElement>(".strip");
  if (!strip) {
    strip = h("div", { class: "strip", title: "Where this file changed, and what sits on its lines. Click to go there." });
    const go = (e: MouseEvent) => {
      const rect = strip!.getBoundingClientRect();
      const frac = Math.max(0, Math.min(1, (e.clientY - rect.top) / rect.height));
      body.scrollTop = frac * body.scrollHeight - body.clientHeight / 2;
    };
    strip.addEventListener("mousedown", (e) => {
      e.preventDefault();
      go(e);
      const move = (m: MouseEvent) => go(m);
      window.addEventListener("mousemove", move);
      window.addEventListener("mouseup", () => window.removeEventListener("mousemove", move), { once: true });
    });
    body.addEventListener("scroll", stripView);
    pane.append(strip);
  }
  const r = st.review;
  if (!r || !st.file || !st.rows.length) {
    strip.hidden = true;
    return;
  }
  strip.hidden = false;
  strip.style.top = `${body.offsetTop}px`;
  strip.style.height = `${body.clientHeight}px`;
  strip.style.right = `${body.offsetWidth - body.clientWidth}px`;
  const findings = new Map<number, "verdict" | "warn">();
  for (const it of r.items) {
    if (it.path !== st.file.path) continue;
    for (const f of it.findings) {
      if (f.row == null || f.level === "note" || findings.get(f.row) === "verdict") continue;
      findings.set(f.row, f.level);
    }
  }
  const comments = new Set(st.file.comments.flatMap((c) => range(c.start, c.end)));
  const n = st.rows.length;
  replace(
    strip,
    stripMarks(st.rows, st.sel, findings, comments, st.search?.hits ?? []).map((m) =>
      h("i", { class: m.kind, style: `top:${(100 * m.from) / n}%;height:max(3px, ${(100 * (m.to - m.from + 1)) / n}%)` }),
    ),
    h("b", { class: "view" }),
  );
  stripView();
}

/** Moves the strip's box to the part of the file in view. */
function stripView(): void {
  const body = $("#code .body");
  const view = $("#code .strip").querySelector<HTMLElement>(".view");
  if (!view || !body.scrollHeight) return;
  view.style.top = `${(100 * body.scrollTop) / body.scrollHeight}%`;
  view.style.height = `${(100 * body.clientHeight) / body.scrollHeight}%`;
}

// ------------------------------------------------------------------ peek at a name

const PEEK_DELAY = 450;
let peekTimer = 0;
let peekSeq = 0;
let peekKey = "";

/** After the pointer rests on a name, shows what it is. */
function peekLater(e: MouseEvent, line: number, text: string): void {
  const col = caretColumn(e, text);
  const w = col === null ? null : wordAt(text, col);
  const key = w ? `${line}:${w.start}` : "";
  if (key === peekKey) return;
  peekKey = key;
  clearTimeout(peekTimer);
  peekSeq++;
  if (!$("#peek").matches(":hover")) hidePeek();
  if (!w) return;
  const at = { x: e.clientX, y: e.clientY };
  peekTimer = window.setTimeout(() => showPeek(line, w.word, w.start, at), PEEK_DELAY);
}

function peekLeave(): void {
  clearTimeout(peekTimer);
  peekSeq++;
  peekKey = "";
  setTimeout(() => {
    if (!$("#peek").matches(":hover")) hidePeek();
  }, 150);
}

function hidePeek(): void {
  const p = $("#peek");
  p.hidden = true;
  replace(p);
}

async function showPeek(line: number, word: string, col: number, at: { x: number; y: number }): Promise<void> {
  const r = st.review;
  if (!r) return;
  const seq = ++peekSeq;
  // hunks that declare the name or change its body, other than the one being read
  const changed = r.view.filter((i) => i !== st.sel && (r.items[i].symbols.includes(word) || r.items[i].enclosing === word));
  let found: string[] = [];
  try {
    const out = await api.act({ op: "hover", brief: true, sel: st.sel, generation: r.generation, line: line - 1, col });
    found = out.popup?.lines ?? [];
  } catch {
    // the review moved on underneath; the hunks it changes still say enough
  }
  if (seq !== peekSeq) return;
  if (!found.length && !changed.length) return;
  const shown = changed.slice(0, 3);
  const p = $("#peek");
  replace(
    p,
    h("header", null, word),
    found.length ? h("pre", null, found.join("\n").trim()) : null,
    changed.length
      ? h(
          "div",
          { class: "changed" },
          h("h4", null, changed.length === 1 ? "Changed in this review" : `Changed in ${changed.length} hunks of this review`),
          shown.map((i) =>
            h(
              "button",
              { onclick: () => (hidePeek(), jumpTo(i)) },
              h("span", { class: "at" }, `${r.items[i].path}:${r.items[i].new[0]}`),
              h("span", null, sentence(r.items[i].rationale)),
            ),
          ),
          changed.length > shown.length ? h("p", { class: "more" }, `and ${changed.length - shown.length} more`) : null,
        )
      : null,
  );
  p.hidden = false;
  const w = p.offsetWidth;
  const ht = p.offsetHeight;
  p.style.left = `${Math.max(8, Math.min(at.x + 12, innerWidth - w - 8))}px`;
  p.style.top = `${at.y + 18 + ht < innerHeight ? at.y + 18 : Math.max(8, at.y - ht - 12)}px`;
}

/** The character the pointer is over in a code line, for `K` on a name. */
function caretColumn(e: MouseEvent, text: string): number | null {
  const doc = document as Document & { caretPositionFromPoint?: (x: number, y: number) => { offsetNode: Node; offset: number } | null };
  const pos = doc.caretPositionFromPoint?.(e.clientX, e.clientY);
  const line = (e.currentTarget as HTMLElement).lastElementChild;
  if (!pos || !line || !line.contains(pos.offsetNode)) return null;
  const walker = document.createTreeWalker(line, NodeFilter.SHOW_TEXT);
  let at = 0;
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    if (n.parentElement?.closest(".gone")) {
      // a click on removed text lands where it stood
      if (n === pos.offsetNode) return Math.min(at, [...text].length);
      continue;
    }
    if (n === pos.offsetNode) return Math.min(at + [...(n.textContent ?? "").slice(0, pos.offset)].length, [...text].length);
    at += [...(n.textContent ?? "")].length;
  }
  return null;
}

/** The caret's column on a line, kept inside it; -1 when no column is set. */
function caretCol(text: string): number {
  if (st.col === null) return -1;
  return Math.min(st.col, Math.max(0, [...text].length - 1));
}

function firstWordCol(text: string): number {
  const m = /[A-Za-z_]/.exec(text);
  return m ? [...text.slice(0, m.index)].length : 0;
}

function openEditor(): void {
  const r = st.review!;
  const it = r.items[st.sel];
  if (!it) return;
  const row = st.rows[st.cursor];
  const line = row && row.kind !== "del" ? row.n : it.new[0];
  if (r.editor === "server") {
    act({ op: "open-editor" });
    return;
  }
  const segments = (p: string) => p.split("/").map(encodeURIComponent).join("/");
  const url = (r.editor || "vscode://file/{root}/{path}:{line}")
    .replaceAll("{root}", segments(r.root).replace(/^\//, ""))
    .replaceAll("{path}", segments(it.path))
    .replaceAll("{line}", String(line));
  window.open(url, "_self");
}

function range(a: number, b: number): number[] {
  return Array.from({ length: Math.max(0, b - a + 1) }, (_, k) => a + k);
}

function moveCursor(by: number): void {
  cursorToRow(Math.max(0, Math.min(st.rows.length - 1, st.cursor + by)));
}

/** Puts the code cursor on row `k`, keeping its column, and follows it into the hunk there. */
function cursorToRow(k: number): void {
  st.cursor = k;
  const hunk = st.rows[st.cursor]?.hunk;
  if (hunk !== null && hunk !== undefined && hunk !== st.sel) {
    select(hunk, { keepCursor: true }).then(followCursor);
    return;
  }
  renderCode();
  followCursor();
}

function cursorTo(pred: (row: CodeRow, k: number) => boolean, forward: boolean): void {
  const n = st.rows.length;
  for (let d = 1; d < n; d++) {
    const k = forward ? st.cursor + d : st.cursor - d;
    if (k < 0 || k >= n) break;
    if (pred(st.rows[k], k)) {
      st.cursor = k;
      renderCode();
      followCursor();
      return;
    }
  }
}

// ------------------------------------------------------------------ why

function edgeRows(targets: { target: number | null; label: string }[], limit: number): HTMLElement[] {
  const r = st.review!;
  return targets.slice(0, limit).map(({ target, label }) => {
    if (target === null || !r.items[target]) {
      return h("div", { class: "link out" }, h("span", { class: "who" }, label), h("span", { class: "what" }, "Not part of this review"));
    }
    const t = r.items[target];
    const link = st.whyLinks.length;
    st.whyLinks.push(target);
    return h(
      "div",
      {
        class: `link${st.whyCur === link ? " cur" : ""}`,
        title: "Go there (Enter); come back with C-o",
        onclick: () => jumpTo(target),
      },
      h("span", { class: "who" }, t.symbols[0] ?? t.enclosing ?? t.symbol ?? label ?? basename(t.path)),
      h("span", { class: "loc" }, `${basename(t.path)}:${t.new[0]}`),
      h("span", { class: "what" }, prose(sentence(t.rationale))),
    );
  });
}

function basename(p: string): string {
  return p.slice(p.lastIndexOf("/") + 1);
}

function table(title: string, hint: string, targets: { target: number | null; label: string }[], empty: string): (HTMLElement | false)[] {
  const limit = tableLimit(targets.length, st.zoom && st.focus === "why");
  const more = targets.length - limit;
  return [
    h("h3", null, title, hint && h("small", null, hint)),
    targets.length ? h("div", { class: "links" }, edgeRows(targets, limit)) : h("p", { class: "muted" }, empty),
    more > 0 &&
      h(
        "button",
        {
          class: "more",
          onclick: () => {
            st.focus = "why";
            st.zoom = true;
            renderAll();
          },
        },
        `Show ${more} more`,
      ),
  ];
}

function rulesChecked(r: Review, it: Item): string {
  const { lang, catalog, own } = it.rules_for;
  const n = (k: number) => `${k} rule${k === 1 ? "" : "s"}`;
  const parts: string[] = [];
  parts.push(r.rules.catalog ? `${n(catalog)} from the built-in catalog` : "no built-in catalog (switched off)");
  const presets = r.rules.includes.length ? `, with ${r.rules.includes.join(", ")}` : "";
  if (own) parts.push(`${n(own)} from your rules files${presets}`);
  return `${n(catalog + own)} that apply to ${lang ?? "this file"}: ${parts.join(" and ")}`;
}

function renderWhy(): void {
  const pane = $("#why");
  const r = st.review!;
  const it = r.items[st.sel];
  replace(pane.querySelector(".head")!, h("span", { class: "t" }, "Why it is here"), h("span", { class: "grow" }), zoomButton("why"));
  const body = pane.querySelector(".body")!;
  if (!it) return replace(body);
  st.whyLinks = [];
  const facts = st.facts && st.facts.i === st.sel ? st.facts : null;
  const needs = it.edges.filter((e) => e.needs);
  const neededBy = it.edges.filter((e) => !e.needs);
  const findings = [...it.findings].sort((a, b) => LEVELS.indexOf(a.level) - LEVELS.indexOf(b.level));
  const churn = r.file_churn[it.path];
  const lc = facts?.line_churn;
  const top = body.scrollTop;
  replace(
    body,
    h(
      "div",
      { class: "why" },
      it.note && [h("h3", null, "Your note"), h("p", { class: "warnline" }, it.note)],
      !!facts?.comments.length && [
        h("h3", null, "Your comments"),
        facts.comments.map((c) => h("p", { class: "warnline" }, `Line ${c.start}${c.end > c.start ? `-${c.end}` : ""}: ${c.text}${c.stale ? " (the lines changed since)" : ""}`)),
      ],
      !!facts?.out_of_order.length && [
        h("h3", null, "Read too early", h("small", null, "it depends on hunks not read yet")),
        h("div", { class: "links" }, edgeRows(facts.out_of_order.map((t) => ({ target: t, label: "" })), facts.out_of_order.length)),
      ],
      table("Needs", "what this hunk uses", needs, "Nothing in this change."),
      table("Needed by", "what uses this hunk", neededBy, "Nothing in this change."),
      facts &&
        table(
          "If you reject it",
          "what loses its footing, in reading order",
          facts.cascade.map((t) => ({ target: t, label: "" })),
          "Rejecting it strands nothing.",
        ),
      h("h3", null, "Findings", findings.length > 0 && h("small", null, "the most pressing first")),
      findings.length
        ? findings.map((f) =>
            h(
              "div",
              { class: "finding" },
              h(
                "div",
                { class: "h" },
                h("span", { class: `lv lv-${f.level}` }, sentence(f.level)),
                h("span", { class: "name" }, f.name),
                f.row != null && h("button", { class: "at", onclick: () => cursorToLine(f.row!), title: "Put the cursor on this line" }, `line ${f.row}`),
              ),
              h("p", null, f.message),
              h("div", { class: "src" }, f.source + (f.original_level ? `; ${f.original_level} in its rule, changed by rules.toml` : "")),
            ),
          )
        : h("p", { class: "muted" }, `None. Checked against ${rulesChecked(r, it)}.`),
      it.executed && [
        h("h3", null, "Test coverage"),
        h(
          "p",
          { class: it.executed[0] === it.executed[1] ? "" : "warnline" },
          it.executed[0] === it.executed[1]
            ? `All ${it.executed[1]} executable lines here run under a test.`
            : `${it.executed[1] - it.executed[0]} of ${it.executed[1]} executable lines here run under no test.`,
        ),
      ],
      (churn || lc) && [
        h("h3", null, "History"),
        churn !== undefined && churn > 0 && h("p", null, `${churn} commit${churn === 1 ? "" : "s"} touched this file in the last six months.`),
        lc &&
          h(
            "p",
            null,
            `These lines changed ${lc.commits}${lc.capped ? "+" : ""} time${lc.commits === 1 ? "" : "s"} before`,
            lc.last ? `, last by ${lc.last.author} on ${lc.last.date}.` : ".",
          ),
      ],
      it.notes.length > 0 && [h("h3", null, "Also"), it.notes.map((n) => h("p", null, prose(sentence(n))))],
      it.wave !== null && [
        h("h3", null, "Wave"),
        h("p", null, `Wave ${it.wave}${r.wave_intents[String(it.wave)] ? `, asked for: ${r.wave_intents[String(it.wave)]}` : ""}`),
      ],
    ),
  );
  body.scrollTop = top;
}

function cursorToLine(n: number): void {
  const k = st.rows.findIndex((row) => row.kind !== "del" && row.n === n);
  if (k < 0) return;
  st.cursor = k;
  st.focus = "code";
  renderAll();
  followCursor();
}

function jumpTo(target: number): void {
  st.jumps.push({ sel: st.sel, cursor: st.cursor });
  select(target, { focus: "code" });
}

function jumpBack(): void {
  const j = st.jumps.pop();
  if (!j) return;
  select(j.sel, { focus: "code" }).then(() => {
    st.cursor = Math.min(j.cursor, st.rows.length - 1);
    renderCode();
    followCursor();
  });
}

// ------------------------------------------------------------------ band, hints, sheets

function renderBand(): void {
  const band = $("#band");
  const r = st.review;
  const help = h("button", { class: "sig help", onclick: () => showHelp(), title: "Every key and command" }, "Keys");
  if (!r) {
    return replace(
      band,
      h("span", { class: "mark" }, "ordo"),
      h("span", { class: "rev" }, rev),
      h("span", { class: "subject" }, st.status),
      st.empty && h("button", { class: "sig", onclick: () => act({ op: "reload" }) }, "Read again"),
    );
  }
  const sinceCount = r.delta.new + r.delta.changed + r.delta.moved;
  const plural: Record<Level, [string, string]> = { verdict: ["verdict", "verdicts"], warn: ["warning", "warnings"], note: ["note", "notes"] };
  replace(
    band,
    h("span", { class: "mark" }, "ordo"),
    h("span", { class: "rev", title: "the revision under review" }, r.rev),
    h("span", { class: "subject", title: r.subject ?? "" }, r.subject ?? (r.uncommitted ? "Uncommitted work" : "")),
    r.stale && h("button", { class: "sig stale", onclick: () => act({ op: "reload" }), title: "Read the working tree again" }, r.stale),
    r.watch !== "off" &&
      h(
        "span",
        { class: "sig", title: r.watch === "auto" ? "--watch=auto: read again once the working tree settles and you pause" : "--watch=hint: says when the working tree has moved on" },
        r.watch === "auto" ? "Watching, reloads" : "Watching",
      ),
    !r.delta.first_run &&
      sinceCount > 0 &&
      h(
        "button",
        {
          class: "sig",
          "aria-pressed": st.sinceOnly ? "true" : "false",
          title: `${r.delta.new} new, ${r.delta.changed} changed, ${r.delta.moved} moved, ${r.delta.gone} gone. Show only these.`,
          onclick: () => {
            st.sinceOnly = !st.sinceOnly;
            renderAll();
          },
        },
        h("b", null, sinceCount),
        "changed since your last look",
      ),
    LEVELS.filter((l) => r.levels[l]).map((l) =>
      h(
        "button",
        { class: `sig lv-${l}`, title: `Go to the next ${plural[l][0]}`, onclick: () => act({ op: "action", action: "finding-next" }) },
        h("b", null, r.levels[l]),
        plural[l][r.levels[l] === 1 ? 0 : 1],
      ),
    ),
    h(
      "button",
      {
        class: "sig progress",
        "aria-pressed": st.summary ? "true" : "false",
        title: `${r.coverage.both} of ${r.coverage.edges} dependency links have both ends read. The review on one page (:summary).`,
        onclick: () => toggleSummary(),
      },
      h("span", { class: "said" }, h("b", null, r.coverage.done), ` of ${r.coverage.total} read`),
      h("span", { class: "dimstr", "aria-hidden": "true" }, h("i", { style: `--p:${r.coverage.total ? r.coverage.done / r.coverage.total : 0}` })),
    ),
    help,
  );
}

// ------------------------------------------------------------------ summary

function toggleSummary(): void {
  st.summary = !st.summary;
  renderAll();
  if (st.summary) $("#summary").focus();
}

/** The keys the summary page answers; the rest would act on columns it hides. */
function summaryKey(action: string): boolean {
  const page = $("#summary");
  const by: Record<string, number> = { next: 60, prev: -60, "half-down": 0.5, "half-up": -0.5, "page-down": 1, "page-up": -1 };
  if (action in by) {
    page.scrollTop += Math.abs(by[action]) < 2 ? by[action] * page.clientHeight : by[action];
  } else if (action === "first" || action === "last") {
    page.scrollTop = action === "first" ? 0 : page.scrollHeight;
  } else if (action === "quit") {
    if (!dismissSheet()) toggleSummary();
  } else {
    return !["command", "command-goto", "help", "reload"].includes(action);
  }
  return true;
}

function readFromSummary(i: number): void {
  st.summary = false;
  jumpTo(i);
}

function renderSummary(): void {
  const r = st.review!;
  const page = $("#summary");
  const keep = page.scrollTop;
  const fl = fileLines(r);
  const notes = written(r);
  const serious = flagged(r, ["verdict", "warn"]);
  const n = (k: number, one: string, many = `${one}s`) => `${k} ${k === 1 ? one : many}`;
  const at = (path: string, line: number, end?: number) => `${path}:${line}${end && end > line ? `-${end}` : ""}`;
  const link = (i: number | null, label: string) =>
    i === null ? h("span", { class: "at" }, label) : h("button", { class: "at", title: "Read this hunk", onclick: () => readFromSummary(i) }, label);
  const nComments = notes.filter((w) => w.kind === "comment").length;
  const nNotes = notes.length - nComments;
  const unreadFiles = fl.filter((f) => f.read < f.hunks.length).length;
  const count = (l: Level) => serious.filter((x) => x.f.level === l).length;
  const notesLeft = flagged(r, ["note"]).length;
  const minor = notesLeft ? h("p", { class: "aside" }, `ordo also left ${n(notesLeft, "note")} on hunks; they are in each hunk's margin.`) : null;
  replace(
    page,
    h(
      "article",
      null,
      r.subject ? h("p", { class: "kicker" }, h("span", { class: "path" }, r.rev)) : null,
      h("h1", { class: "headline" }, r.subject ?? (r.uncommitted ? "Uncommitted work" : `Review of ${r.rev}`)),
      h(
        "p",
        { class: "lede" },
        `You have read ${r.coverage.done} of ${n(r.coverage.total, "hunk")}`,
        unreadFiles ? `, with ${n(unreadFiles, "file")} still to finish` : ", every file",
        `. You wrote ${n(nComments, "comment")} and ${n(nNotes, "note")}; ordo flagged ${n(count("verdict"), "verdict")} and ${n(count("warn"), "warning")}.`,
      ),
      h("div", { class: "meter wide" }, h("i", { style: `width:${r.coverage.total ? (100 * r.coverage.done) / r.coverage.total : 0}%` })),
      h(
        "div",
        { class: "actions" },
        h("button", { class: "primary", title: "Your notes and comments as one markdown message (:yank)", onclick: () => act({ op: "command", line_text: "yank" }) }, "Copy for the author"),
        h("button", { title: "The same, with ordo's own notes and findings (:yank all)", onclick: () => act({ op: "command", line_text: "yank all" }) }, "Copy with ordo's findings"),
        h("button", { onclick: () => window.print() }, "Print"),
        h("button", { onclick: () => toggleSummary() }, "Back to reading"),
      ),
      h("h2", null, "What you wrote"),
      notes.length
        ? h(
            "ol",
            { class: "written" },
            notes.map((w) => {
              if (w.kind === "note") {
                const it = r.items[w.i];
                return h("li", null, h("div", { class: "where" }, link(w.i, at(it.path, it.new[0])), h("span", null, `note on ${it.symbol || it.symbols[0] || "this hunk"}`)), h("p", { class: "said" }, w.text));
              }
              const c = w.comment;
              return h(
                "li",
                null,
                h("div", { class: "where" }, link(w.i, at(c.path, c.start, c.end)), c.stale ? h("span", { class: "stale" }, "these lines changed since") : null),
                h("p", { class: "said" }, c.text),
              );
            }),
          )
        : h("p", { class: "aside" }, "Nothing written yet. Select lines and :comment, or :note a symbol."),
      h("h2", null, "What ordo flagged"),
      serious.length
        ? h(
            "ol",
            { class: "flagged" },
            serious.map(({ i, f }) =>
              h(
                "li",
                null,
                h("div", { class: "where" }, h("span", { class: `lv lv-${f.level}` }, f.level === "warn" ? "warning" : f.level), h("b", null, f.name), link(i, at(r.items[i].path, f.row ?? r.items[i].new[0]))),
                h("p", { class: "said" }, prose(f.message.split("\n")[0])),
              ),
            ),
          )
        : h("p", { class: "aside" }, "No verdicts or warnings."),
      minor,
      h("h2", null, "File by file"),
      h(
        "table",
        { class: "files" },
        h("thead", null, h("tr", null, h("th", null, "File"), h("th", null, "Read"), h("th", null, "Comments"), h("th", null, "Flagged"))),
        h(
          "tbody",
          null,
          fl.map((f) => {
            const next = f.hunks.find((i) => !r.items[i].reviewed) ?? f.hunks[0];
            return h(
              "tr",
              { class: f.read === f.hunks.length ? "done" : null },
              h("td", null, link(next, f.path)),
              h("td", null, h("span", { class: "meter" }, h("i", { style: `width:${(100 * f.read) / f.hunks.length}%` })), ` ${f.read} of ${f.hunks.length}`),
              h("td", null, f.comments || ""),
              h("td", null, LEVELS.filter((l) => l !== "note" && f.levels[l]).map((l) => h("span", { class: `lv lv-${l}` }, `${f.levels[l]} ${l === "warn" ? "warning" : l}${f.levels[l] === 1 ? "" : "s"}`))),
            );
          }),
        ),
      ),
    ),
  );
  page.scrollTop = keep;
}

const HINTS: Record<Pane, [string, string][]> = {
  list: [["next", "next hunk"], ["finding-next", "next finding"], ["command", "commands"]],
  code: [["next", "next line"], ["search", "search"], ["select-lines", "select"], ["jump-back", "back"]],
  why: [["next", "next link"], ["jump-to-edge", "go there"], ["jump-back", "back"]],
};

function renderHints(): void {
  const items = HINTS[st.focus].map(([action, label]) => {
    const k = st.keymap.keysFor(action)[0];
    return k ? h("span", null, h("kbd", null, k), label) : null;
  });
  replace($("#hints"), st.notice ? h("span", { class: "notice" }, st.notice) : items);
}



function zoomButton(p: Pane): HTMLElement {
  const on = st.zoom && st.focus === p;
  return h(
    "button",
    {
      class: on ? "zoomer on" : "zoomer",
      title: on ? "Show all three columns" : "Give this column the whole page",
      "aria-label": on ? "restore the columns" : "maximise this column",
      onclick: () => {
        st.zoom = !on;
        st.focus = p;
        renderAll();
      },
    },
    h("i", { "aria-hidden": "true" }),
  );
}

function showSheet(title: string, text: string | Node): void {
  const ov = $("#overlay");
  ov.hidden = false;
  replace(
    ov,
    h(
      "div",
      { class: "sheet", role: "dialog", "aria-label": title },
      h("header", null, title, h("button", { onclick: closeSheet, "aria-label": "close" }, "×")),
      typeof text === "string" ? h("pre", null, text) : h("div", { class: "scroll" }, text),
    ),
  );
}

/** Closes whatever sheet is open; settings are the server's to close. */
function dismissSheet(): boolean {
  if ($("#overlay").dataset.kind === "config") {
    act({ op: "config", do: "close" });
    return true;
  }
  return closeSheet();
}

function closeSheet(): boolean {
  const ov = $("#overlay");
  if (ov.hidden) return false;
  ov.hidden = true;
  delete ov.dataset.kind;
  canvas = null;
  depmap = null;
  replace(ov);
  return true;
}

function showHelp(): void {
  const r = st.review;
  if (!r) return;
  const by = new Map<string, string[]>();
  for (const b of r.keys) {
    const k = b.prefix ? `${b.prefix} ${b.key}` : b.key;
    by.set(b.action, [...(by.get(b.action) ?? []), k]);
  }
  const rows = [...by].map(([a, ks]) => h("tr", null, h("td", null, ks.join(", ")), h("td", null, a)));
  const cmds = [...r.commands, ...CLIENT_COMMANDS].map((c) => h("tr", null, h("td", null, `:${c.name} ${c.args}`), h("td", null, c.help)));
  showSheet("keys and commands", h("div", { class: "why" }, h("table", null, h("tbody", null, rows)), h("h3", null, "commands"), h("table", null, h("tbody", null, cmds))));
}

let toastTimer = 0;
function toast(msg: string, err = false): void {
  const t = $("#toast");
  t.textContent = msg;
  t.className = `show${err ? " err" : ""}`;
  clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => (t.className = ""), err ? 4000 : 2200);
}

function renderAll(): void {
  document.body.classList.toggle("zoom", st.zoom);
  for (const p of PANES) {
    const el = $(`#${p}`);
    el.classList.toggle("focus", st.focus === p);
    el.classList.toggle("zoomed", st.zoom && st.focus === p);
  }
  renderBand();
  document.body.classList.toggle("summary", st.summary && !!st.review);
  $("#summary").hidden = !(st.summary && st.review);
  if (st.summary && st.review) return renderSummary();
  if (!st.review) {
    for (const p of PANES) {
      replace($(`#${p} > .head`));
      replace($(`#${p} > .body`));
    }
    replace($("#code .foot .actions"));
    replace($("#code > .body"), h("div", { class: "loading" }, st.empty ? "Nothing to read in this revision." : "Reading the change…", h("small", null, st.status)));
    renderHints();
    return;
  }
  renderList();
  renderCode();
  renderWhy();
  renderHints();
}

// ------------------------------------------------------------------ palette

const SIZES: Record<string, number> = { small: 0.9, normal: 1, large: 1.15, larger: 1.3 };

/** A `:size` argument as a scale factor: a name, a factor, or a percentage. */
function sizeOf(arg: string): number | null {
  const a = arg.trim();
  if (!a) return 1;
  if (a in SIZES) return SIZES[a];
  const n = a.endsWith("%") ? parseFloat(a) / 100 : parseFloat(a);
  return Number.isFinite(n) && n >= 0.6 && n <= 2.5 ? n : null;
}

function applySize(ui: number): void {
  document.documentElement.style.setProperty("--ui", String(ui));
}

type PaletteMode = "command" | "search";
let paletteMode: PaletteMode = "command";
let paletteCur = 0;
/** whether the arrows picked a candidate, which Enter then runs */
let palettePicked = false;

function openPalette(mode: PaletteMode, text = ""): void {
  paletteMode = mode;
  const form = $<HTMLFormElement>("#palette");
  form.hidden = false;
  $("#palette label").textContent = mode === "command" ? ":" : "/";
  const input = $<HTMLInputElement>("#palette-input");
  input.value = text;
  paletteCur = 0;
  palettePicked = false;
  renderPalette();
  input.focus();
}

function closePalette(): void {
  $<HTMLFormElement>("#palette").hidden = true;
  (document.activeElement as HTMLElement | null)?.blur();
}

function argCandidates(): Record<string, string[]> {
  const r = st.review!;
  return {
    strategy: ["comprehension", "defs-first", "file"],
    mode: ["ledger", "hunks"],
    watch: ["on", "off", "auto"],
    "only-wave": ["last", "all", ...Object.keys(r.wave_intents)],
    yank: ["all"],
    send: ["all"],
    theme: ["auto", ...themeNames()],
    size: Object.keys(SIZES),
    goto: [...new Set(r.items.map((i) => i.path))],
    filter: [...new Set(r.items.map((i) => i.path.replace(/[^/]*$/, "*")))],
  };
}

function candidates(): string[] {
  if (paletteMode !== "command" || !st.review) return [];
  const input = $<HTMLInputElement>("#palette-input").value;
  return complete(input, [...st.review.commands, ...CLIENT_COMMANDS], argCandidates()).slice(0, 40);
}

function renderPalette(): void {
  const list = $("#palette-list");
  if (paletteMode !== "command" || !st.review) return replace(list);
  const all = [...st.review.commands, ...CLIENT_COMMANDS];
  replace(
    list,
    candidates().map((c, k) => {
      const cmd = all.find((x) => x.name === c.split(" ")[0]);
      return h(
        "li",
        {
          class: k === paletteCur ? "cur" : "",
          onmousedown: (e: Event) => {
            e.preventDefault();
            $<HTMLInputElement>("#palette-input").value = c;
            runPalette();
          },
        },
        h("b", null, c),
        h("span", null, cmd && !c.includes(" ") ? cmd.help : ""),
      );
    }),
  );
}

async function runPalette(): Promise<void> {
  const picked = palettePicked && candidates()[paletteCur];
  const value = picked || $<HTMLInputElement>("#palette-input").value;
  closePalette();
  if (paletteMode === "search") return runSearch(value);
  const { name, arg } = parseCommand(value);
  if (!name) return;
  if (name === "theme") {
    applyTheme(arg || "auto");
    localStorage.setItem("ordo.look", arg || "auto");
    return;
  }
  if (name === "map") return openMap();
  if (name === "summary") return toggleSummary();
  if (name === "size") {
    const ui = sizeOf(arg);
    if (ui === null) return toast(`:size takes ${Object.keys(SIZES).join(", ")} or a factor like 1.2`, true);
    applySize(ui);
    localStorage.setItem("ordo.size", String(ui));
    return renderAll();
  }
  const out = await act({ op: "command", line_text: value });
  if (out && !out.error && name === "comment" && st.from !== null) {
    st.from = null;
    renderCode();
  }
}

/** The first hit from the cursor on, or with `forward` off the last one before it. */
function runSearch(query: string, forward = true): void {
  if (!query) {
    st.search = null;
    return renderCode();
  }
  const q = query.toLowerCase();
  const hits = st.rows.flatMap((row, k) => (row.text.toLowerCase().includes(q) ? [k] : []));
  const after = hits.findIndex((x) => x > st.cursor);
  const at = hits.indexOf(st.cursor);
  const k = forward
    ? Math.max(0, at >= 0 ? at : after)
    : Math.max(0, (after < 0 ? hits.length : after) - (at >= 0 ? 2 : 1));
  st.search = { query, hits, k };
  if (hits.length) st.cursor = hits[k];
  st.focus = "code";
  renderAll();
  followCursor();
}

function searchStep(by: number): void {
  const s = st.search;
  if (!s || !s.hits.length) return;
  s.k = (s.k + by + s.hits.length) % s.hits.length;
  st.cursor = s.hits[s.k];
  renderCode();
  followCursor();
}

// ------------------------------------------------------------------ keys

function page(by: number): void {
  const body = $(`#${st.focus} .body`);
  body.scrollTop += by * body.clientHeight;
}

/** One keymap action, as the focused pane takes it. */
function run(action: string): void {
  const r = st.review;
  if (!r) return;
  if (st.summary && summaryKey(action)) return;
  const focus = st.focus;
  switch (action) {
    case "quit":
      if (dismissSheet()) return;
      if (st.search) st.search = null;
      else if (st.from !== null) st.from = null;
      else if (st.zoom) st.zoom = false;
      return renderAll();
    case "help":
      return showHelp();
    case "command":
      return openPalette("command");
    case "command-goto":
      return openPalette("command", "goto ");
    case "search":
      return openPalette("search");
    case "search-next":
      return searchStep(1);
    case "search-prev":
      return searchStep(-1);
    case "symbol-next":
    case "symbol-prev": {
      const it = r.items[st.sel];
      const name = it?.symbols[0] ?? it?.enclosing;
      const by = action === "symbol-next" ? 1 : -1;
      if (name && st.search?.query === name) return searchStep(by);
      if (name) runSearch(name, by > 0);
      return;
    }
    case "next":
    case "prev": {
      const by = action === "next" ? 1 : -1;
      if (focus === "list") return step(by);
      if (focus === "code") return moveCursor(by);
      return whyStep(by);
    }
    case "first":
      if (focus === "list") return void (stops().length && select(stops()[0]));
      if (focus === "code") return moveCursor(-st.rows.length);
      $("#why .body").scrollTop = 0;
      return;
    case "last":
      if (focus === "list") return void (stops().length && select(stops().at(-1)!));
      if (focus === "code") return moveCursor(st.rows.length);
      $("#why .body").scrollTop = 1e9;
      return;
    case "page-down":
      return page(1);
    case "page-up":
      return page(-1);
    case "half-down":
      return page(0.5);
    case "half-up":
      return page(-0.5);
    case "toggle-reviewed": {
      const s = stops();
      const at = s.indexOf(st.sel);
      act({ op: "mark" }).then((out) => {
        if (out && !out.error && at >= 0 && at + 1 < s.length) select(s[at + 1]);
      });
      return;
    }
    case "focus-next":
    case "focus-prev": {
      const k = PANES.indexOf(focus) + (action === "focus-next" ? 1 : PANES.length - 1);
      st.focus = PANES[k % PANES.length];
      return renderAll();
    }
    case "focus-list":
    case "focus-code":
    case "focus-why": {
      const p = action.slice(6) as Pane;
      if (st.focus === p) st.zoom = !st.zoom;
      st.focus = p;
      return renderAll();
    }
    case "zoom":
      st.zoom = !st.zoom;
      return renderAll();
    case "finding-next":
    case "finding-prev":
    case "fold-toggle":
    case "fold-open":
    case "fold-close":
    case "fold-open-all":
    case "fold-close-all":
    case "wave-next":
    case "wave-prev":
      act({ op: "action", action });
      return;
    case "jump-to-edge":
      if (focus === "why" && st.whyLinks[st.whyCur] !== undefined) jumpTo(st.whyLinks[st.whyCur]);
      return;
    case "jump-back":
      return jumpBack();
    case "mark-next":
    case "mark-prev": {
      const uses = new Set(r.items[st.sel].uses_at.flatMap((u) => u.rows));
      return cursorTo((row) => row.kind !== "del" && uses.has(row.n), action === "mark-next");
    }
    case "comment-next":
    case "comment-prev": {
      const c = new Set(st.file?.comments.map((c) => c.start) ?? []);
      return cursorTo((row) => row.kind !== "del" && c.has(row.n), action === "comment-next");
    }
    case "para-next":
    case "para-prev":
      return cursorTo((row) => row.text.trim() === "", action === "para-next");
    case "cursor-left":
    case "cursor-right":
    case "line-start":
    case "line-end": {
      const text = st.rows[st.cursor]?.text ?? "";
      const last = Math.max(0, [...text].length - 1);
      const now = st.col === null ? firstWordCol(text) : caretCol(text);
      st.col =
        action === "line-start" ? 0 : action === "line-end" ? last : Math.max(0, Math.min(last, now + (action === "cursor-right" ? 1 : -1)));
      st.focus = "code";
      return renderAll();
    }
    case "word-next":
    case "word-prev":
    case "word-end": {
      const kind = action === "word-next" ? "next" : action === "word-prev" ? "prev" : "end";
      const text = st.rows[st.cursor]?.text ?? "";
      const to = wordMove(
        st.rows.map((r) => r.text),
        { row: st.cursor, col: st.col === null ? firstWordCol(text) : caretCol(text) },
        kind,
      );
      st.col = to.col;
      st.focus = "code";
      if (to.row === st.cursor) return renderAll();
      return cursorToRow(to.row);
    }
    case "select-lines":
      st.from = st.from === null ? st.cursor : null;
      st.focus = "code";
      return renderAll();
    case "comment":
      return openPalette("command", "comment ");
    case "reload":
      act({ op: "reload" });
      return;
    case "hover":
      if (focus === "why" && st.whyLinks[st.whyCur] !== undefined) return void previewHunk(st.whyLinks[st.whyCur]);
      act({ op: "hover", col: st.col === null ? firstWordCol(st.rows[st.cursor]?.text ?? "") : caretCol(st.rows[st.cursor]?.text ?? "") });
      return;
    case "open-editor":
      return openEditor();
    case "open-deps":
    case "focus-deps":
      return void openCanvas();
  }
}

function whyStep(by: number): void {
  const n = st.whyLinks.length;
  if (!n) {
    $("#why .body").scrollTop += by * 40;
    return;
  }
  st.whyCur = Math.max(0, Math.min(n - 1, st.whyCur + by));
  renderWhy();
  $("#why tr.cur")?.scrollIntoView({ block: "nearest" });
}

function onKey(e: KeyboardEvent): void {
  if (!$("#peek").hidden) hidePeek();
  const t = e.target as HTMLElement | null;
  if (t && (t.tagName === "INPUT" || t.tagName === "SELECT" || t.tagName === "TEXTAREA") && t.id !== "palette-input") return;
  // a focused button takes Enter and Space as a click, not as a motion
  if (t?.tagName === "BUTTON" && (e.key === "Enter" || e.key === " ")) return;
  const paletteOpen = !$<HTMLFormElement>("#palette").hidden;
  if (paletteOpen) return paletteKey(e);
  const key = keyLabel(e);
  if (!key) return;
  // Enter on a why link goes there whatever the preset calls it
  if (key === "Enter" && canvas) {
    e.preventDefault();
    return void canvasKey("jump-to-edge");
  }
  if (key === "Enter" && depmap) {
    e.preventDefault();
    return void mapKey("jump-to-edge");
  }
  if (key === "Esc" && $("#overlay").dataset.kind === "config") {
    e.preventDefault();
    return void dismissSheet();
  }
  if (key === "Enter" && st.focus === "why" && st.whyCur >= 0) {
    e.preventDefault();
    return run("jump-to-edge");
  }
  const pending = st.pending;
  const res = st.keymap.resolve(pending, key);
  clearTimeout(st.pendingTimer);
  st.pending = null;
  if ("pending" in res) {
    e.preventDefault();
    st.pending = key;
    // a prefix that is also a key of its own acts once no chord follows
    const bare = st.keymap.bare(key);
    if (bare) st.pendingTimer = window.setTimeout(() => ((st.pending = null), run(bare)), CHORD_TIMEOUT);
    return;
  }
  if ("act" in res) {
    e.preventDefault();
    if (canvasKey(res.act) || mapKey(res.act)) return;
    return run(res.act);
  }
  if (pending) {
    const bare = st.keymap.bare(pending);
    if (bare) run(bare);
    onKey(e);
  }
}

function paletteKey(e: KeyboardEvent): void {
  const c = candidates();
  if (e.key === "Escape") {
    e.preventDefault();
    return closePalette();
  }
  if (e.key === "Tab" || e.key === "ArrowDown" || e.key === "ArrowUp") {
    e.preventDefault();
    if (!c.length) return;
    if (e.key === "Tab") {
      $<HTMLInputElement>("#palette-input").value = c[paletteCur] + (c[paletteCur].includes(" ") ? "" : " ");
      paletteCur = 0;
    } else {
      paletteCur = (paletteCur + (e.key === "ArrowDown" ? 1 : c.length - 1)) % c.length;
      palettePicked = true;
    }
    return renderPalette();
  }
}

// ------------------------------------------------------------------ canvas, previews, config

/** A hunk's own rows, with a little of the file around them, read-only. */
function excerpt(f: FileData, hunk: number, context: number, max: number): HTMLElement {
  if (f.missing) return h("p", { class: "muted" }, "no text to show for this file");
  const rows = codeRows(f);
  const ks = rows.flatMap((r, k) => (r.hunk === hunk ? [k] : []));
  const from = Math.max(0, (ks[0] ?? 0) - context);
  const to = Math.min(rows.length, Math.min((ks.at(-1) ?? 0) + context + 1, from + max));
  return h(
    "div",
    { class: "code" },
    rows.slice(from, to).map((row) =>
      h(
        "div",
        { class: `l ${row.kind}${row.hunk !== null && row.hunk !== hunk ? " other" : ""}` },
        h("span", { class: "g" }),
        h("span", { class: "ln" }, row.kind === "del" ? "" : row.n),
        h("span", { class: "op" }, OP[row.kind]),
        h(
          "span",
          null,
          runs(row.text, row.spans, row.marks, row.kind === "edit" ? row.gone : []).map((run) =>
            run.gone ? h("del", { class: "gone" }, run.text) : h(run.changed ? "mark" : "span", { class: run.role ? `s-${run.role}` : null }, run.text),
          ),
        ),
      ),
    ),
  );
}

function hunkTitle(i: number): string {
  const t = st.review!.items[i];
  return `${t.symbols[0] ?? t.enclosing ?? t.symbol ?? basename(t.path)} in ${basename(t.path)}:${t.new[0]}`;
}

async function previewHunk(i: number): Promise<void> {
  const t = st.review!.items[i];
  const f = await fileOf(t.path);
  showSheet(hunkTitle(i), h("div", null, h("p", { class: "muted" }, t.rationale), h("div", { class: "src" }, excerpt(f, i, 2, 40))));
}

interface Canvas {
  anchor: number;
  cards: { i: number; needs: boolean }[];
  sel: number;
}
let canvas: Canvas | null = null;

async function openCanvas(): Promise<void> {
  const r = st.review!;
  const it = r.items[st.sel];
  const cards = it.edges.filter((e) => e.target !== null && r.items[e.target]).map((e) => ({ i: e.target!, needs: e.needs }));
  if (!cards.length) return toast("nothing in this change needs this hunk, and it needs nothing");
  canvas = { anchor: st.sel, cards, sel: 0 };
  await renderCanvas();
}

async function renderCanvas(): Promise<void> {
  if (!canvas) return;
  const c = canvas;
  const files = await Promise.all([c.anchor, ...c.cards.map((k) => k.i)].map((i) => fileOf(st.review!.items[i].path)));
  if (canvas !== c) return;
  const card = (k: number) => {
    const { i } = c.cards[k];
    return h(
      "div",
      { class: `card${k === c.sel ? " cur" : ""}`, onclick: () => goCanvas(k) },
      h("header", null, hunkTitle(i), h("span", { class: "muted" }, st.review!.items[i].rationale)),
      excerpt(files[k + 1], i, 1, 14),
    );
  };
  const side = (needs: boolean) =>
    h(
      "div",
      { class: "side" },
      h("h3", null, needs ? "Needs: what this hunk uses" : "Needed by: what uses this hunk"),
      c.cards.map((x, k) => (x.needs === needs ? card(k) : null)),
    );
  showSheet(
    `dependencies of ${hunkTitle(c.anchor)}`,
    h(
      "div",
      { class: "canvas src" },
      h("div", { class: "anchor" }, excerpt(files[0], c.anchor, 1, 16)),
      h("div", { class: "sides" }, side(true), side(false)),
    ),
  );
  $("#overlay").querySelector(".sheet")?.classList.add("wide");
  $("#overlay .card.cur")?.scrollIntoView({ block: "nearest" });
}

function goCanvas(k: number): void {
  if (!canvas) return;
  const target = canvas.cards[k].i;
  canvas = null;
  closeSheet();
  jumpTo(target);
}

function canvasKey(action: string): boolean {
  if (!canvas) return false;
  const n = canvas.cards.length;
  if (action === "next" || action === "prev") {
    canvas.sel = (canvas.sel + (action === "next" ? 1 : n - 1)) % n;
    renderCanvas();
  } else if (action === "jump-to-edge") {
    goCanvas(canvas.sel);
  } else if (action === "open-deps" || action === "focus-deps") {
    // the next step out from one hop is the whole chain
    const root = canvas.anchor;
    canvas = null;
    depmap = { root, sel: root, order: [] };
    renderMap();
  } else if (action === "quit") {
    canvas = null;
    closeSheet();
  }
  return true;
}

/** One hunk's dependency chain; `sel` is the hunk picked on it. */
let depmap: { root: number; sel: number; order: number[] } | null = null;

function openMap(): void {
  const r = st.review!;
  if (!needPairs(r.items, r.view).some((p) => p.includes(st.sel))) {
    return toast("nothing in this change needs this hunk, and it needs nothing");
  }
  depmap = { root: st.sel, sel: st.sel, order: [] };
  renderMap();
}

/** Draws the map; `again` is a redraw of one already open, which keeps its scroll. */
function renderMap(again = false): void {
  if (!depmap) return;
  const r = st.review!;
  const root = depmap.root;
  if (!r.view.includes(root)) {
    closeSheet();
    return;
  }
  const scrolled = $("#overlay").querySelector(".sheet > .scroll");
  const keep = again && scrolled ? { top: scrolled.scrollTop, left: scrolled.scrollLeft } : null;
  const ui = parseFloat(getComputedStyle(document.documentElement).getPropertyValue("--ui")) || 1;
  const box = { width: 236 * ui, height: 54 * ui };
  const m = depMap(r.items, r.view, root, box);
  depmap.order = r.view.filter((i) => m.nodes.has(i));
  if (!m.nodes.has(depmap.sel)) depmap.sel = root;
  const pick = (i: number) => {
    if (!depmap || depmap.sel === i) return;
    depmap.sel = i;
    paintMap();
  };
  const node = (i: number, at: { x: number; y: number }) => {
    const it = r.items[i];
    const name = it.symbols[0] ?? it.enclosing ?? it.symbol ?? basename(it.path);
    return h(
      "button",
      {
        class: `node${i === root ? " root" : ""}`,
        dataset: { i: String(i) },
        style: `left:${at.x - box.width / 2}px;top:${at.y - box.height / 2}px;width:${box.width}px;height:${box.height}px`,
        title: it.rationale,
        onmouseenter: () => pick(i),
        onfocus: () => pick(i),
        onclick: () => goMap(i),
      },
      h("span", { class: "name" }, name),
      it.reviewed && h("span", { class: "done" }, "read"),
      h("span", { class: "at" }, `${basename(it.path)}:${it.new[0]}`),
      it.level && h("span", { class: `lv lv-${it.level}` }, it.level === "warn" ? "warning" : it.level),
    );
  };
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("width", String(m.width));
  svg.setAttribute("height", String(m.height));
  svg.setAttribute("aria-hidden", "true");
  svg.innerHTML =
    '<defs><marker id="map-head" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="7" markerHeight="7" orient="auto-start-reverse">' +
    '<path d="M0 0 L8 4 L0 8 z" fill="context-stroke"/></marker></defs>';
  for (const e of m.edges) {
    const p = document.createElementNS("http://www.w3.org/2000/svg", "path");
    p.setAttribute("d", smoothPath(e.points));
    p.setAttribute("marker-end", "url(#map-head)");
    p.dataset.from = String(e.from);
    p.dataset.to = String(e.to);
    if (e.outOfOrder) p.classList.add("early");
    svg.append(p);
  }
  const ground = groundOf(r.items, r.view, root).size;
  const strands = cascadeOf(r.items, r.view, root).size;
  const hunks = (n: number) => `${n} hunk${n === 1 ? "" : "s"}`;
  const early = m.edges.filter((e) => e.outOfOrder).length;
  showSheet(
    `Dependencies of ${hunkTitle(root)}`,
    h(
      "div",
      { class: "mapview" },
      h(
        "p",
        { class: "lede" },
        `It stands on ${hunks(ground)}, drawn to its left, and ${hunks(strands)} ${strands === 1 ? "stands" : "stand"} on it, drawn to its right.`,
        early ? h("span", { class: "early" }, ` ${hunks(early)} marked reviewed before what ${early === 1 ? "it needs" : "they need"}, dashed.`) : null,
      ),
      h("p", { class: "said", "aria-live": "polite" }),
      h("div", { class: "depmap", style: `width:${m.width}px;height:${m.height}px` }, svg, [...m.nodes].map(([i, at]) => node(i, at))),
    ),
  );
  $("#overlay").querySelector(".sheet")?.classList.add("wide");
  $("#overlay").dataset.kind = "map";
  paintMap();
  const scroll = $("#overlay").querySelector(".sheet > .scroll");
  if (keep && scroll) {
    scroll.scrollTop = keep.top;
    scroll.scrollLeft = keep.left;
  } else {
    $("#overlay .node.root")?.scrollIntoView({ block: "center", inline: "center" });
  }
}

/** Marks the picked hunk, what it needs, and what rejecting it would strand. */
function paintMap(): void {
  if (!depmap) return;
  const r = st.review!;
  const i = depmap.sel;
  const strands = cascadeOf(r.items, r.view, i);
  const needs = new Set(needPairs(r.items, r.view).flatMap(([from, to]) => (to === i ? [from] : [])));
  for (const el of $("#overlay").querySelectorAll<HTMLElement>(".node")) {
    const k = Number(el.dataset.i);
    el.classList.toggle("picked", k === i);
    el.classList.toggle("strands", strands.has(k));
    el.classList.toggle("needed", needs.has(k));
    el.classList.toggle("reviewed", r.items[k].reviewed);
  }
  for (const p of $("#overlay").querySelectorAll<SVGPathElement>(".depmap path[data-from]")) {
    const from = Number(p.dataset.from);
    const to = Number(p.dataset.to);
    p.classList.toggle("strands", (from === i || strands.has(from)) && strands.has(to));
    p.classList.toggle("needed", to === i);
  }
  const said = $("#overlay").querySelector(".mapview .said");
  if (said && r.items[i]) {
    const name = hunkTitle(i);
    const stranded = strands.size ? `rejecting it strands ${strands.size} hunk${strands.size === 1 ? "" : "s"}, shaded red` : "nothing here needs it";
    const needed = needs.size ? `; it needs ${needs.size}, outlined` : "";
    said.textContent = `${i === depmap.root ? "This hunk" : name}: ${stranded}${needed}. Click a hunk to read it; gD on it draws its own chain.`;
  }
}

function goMap(i: number): void {
  depmap = null;
  closeSheet();
  jumpTo(i);
}

function mapKey(action: string): boolean {
  if (!depmap || action === "command" || action === "command-goto") return false;
  const order = depmap.order;
  if (action === "next" || action === "prev") {
    const at = order.indexOf(depmap.sel);
    depmap.sel = order[Math.max(0, Math.min(order.length - 1, at + (action === "next" ? 1 : -1)))];
    paintMap();
    $("#overlay .node.picked")?.scrollIntoView({ block: "nearest", inline: "nearest" });
  } else if (action === "jump-to-edge") {
    goMap(depmap.sel);
  } else if ((action === "open-deps" || action === "focus-deps") && depmap.sel !== depmap.root) {
    depmap.root = depmap.sel;
    renderMap();
  } else if (action === "quit" || action === "open-deps" || action === "focus-deps") {
    depmap = null;
    closeSheet();
  }
  return true;
}

/** `:config`, as the server's `ConfigUi` holds it. */
function renderConfig(): void {
  const cfg = st.review?.config;
  const ov = $("#overlay");
  if (!cfg) {
    if (ov.dataset.kind === "config") closeSheet();
    return;
  }
  const step = (body: Record<string, unknown>) => act({ op: "config", ...body });
  const field = (f: NonNullable<typeof cfg>["sections"][number]["fields"][number]) => {
    let control: HTMLElement;
    if (f.kind === "flag") {
      control = h("input", { type: "checkbox", checked: f.value === true, onchange: () => step({ do: "toggle", row: f.row }) });
    } else if (f.kind === "choice") {
      control = h(
        "button",
        { class: "choice", title: (f.options ?? []).join(", "), onclick: () => step({ do: "toggle", row: f.row }) },
        String(f.value ?? ""),
      );
    } else {
      control = h("input", {
        value: String(f.value ?? ""),
        placeholder: "(terminal)",
        onchange: (e: Event) => step({ do: "edit", row: f.row, text: (e.target as HTMLInputElement).value }),
      });
    }
    return h("tr", null, h("td", null, f.label), h("td", null, control), h("td", { class: "muted" }, f.help));
  };
  const body = h(
    "div",
    { class: "why config" },
    cfg.sections.map((s) => [h("h3", null, s.title, h("small", null, s.file)), h("table", null, h("tbody", null, s.fields.map(field)))]),
  );
  const top = ov.querySelector(".scroll")?.scrollTop ?? 0;
  showSheet(cfg.dirty ? "Settings, changed and not written" : "Settings", body);
  ov.dataset.kind = "config";
  const header = ov.querySelector(".sheet > header")!;
  header.insertBefore(h("button", { class: "write", disabled: !cfg.dirty, onclick: () => step({ do: "write" }) }, "Write to files"), header.lastChild);
  header.lastElementChild!.addEventListener("click", () => step({ do: "close" }));
  ov.querySelector(".scroll")!.scrollTop = top;
}

// ------------------------------------------------------------------ wiring

function wireSplits(): void {
  const panes = $("#panes");
  let saved: [string, string] | null = null;
  try {
    saved = JSON.parse(localStorage.getItem("ordo.splits") ?? "null");
  } catch {
    localStorage.removeItem("ordo.splits");
  }
  if (saved) {
    panes.style.setProperty("--c0", saved[0]);
    panes.style.setProperty("--c2", saved[1]);
  }
  for (const split of document.querySelectorAll<HTMLElement>(".split")) {
    split.addEventListener("pointerdown", (e) => {
      split.setPointerCapture(e.pointerId);
      split.classList.add("drag");
      const which = split.dataset.split;
      const move = (ev: PointerEvent) => {
        const box = panes.getBoundingClientRect();
        const x = ((ev.clientX - box.left) / box.width) * 100;
        if (which === "0") panes.style.setProperty("--c0", `${Math.min(60, Math.max(12, x))}%`);
        else panes.style.setProperty("--c2", `${Math.min(60, Math.max(14, 100 - x))}%`);
      };
      const up = () => {
        split.classList.remove("drag");
        split.removeEventListener("pointermove", move);
        localStorage.setItem("ordo.splits", JSON.stringify([panes.style.getPropertyValue("--c0") || "27%", panes.style.getPropertyValue("--c2") || "30%"]));
        if (st.review) renderList();
      };
      split.addEventListener("pointermove", move);
      split.addEventListener("pointerup", up, { once: true });
    });
  }
}

function wire(): void {
  document.addEventListener("keydown", onKey);
  for (const p of PANES) {
    $(`#${p}`).addEventListener("mousedown", () => {
      if (st.focus !== p) {
        st.focus = p;
        renderHints();
        for (const q of PANES) $(`#${q}`).classList.toggle("focus", q === p);
      }
    });
  }
  const form = $<HTMLFormElement>("#palette");
  form.addEventListener("submit", (e) => {
    e.preventDefault();
    runPalette();
  });
  $("#palette-input").addEventListener("input", () => {
    paletteCur = 0;
    palettePicked = false;
    renderPalette();
  });
  $("#palette-input").addEventListener("blur", () => setTimeout(() => (form.hidden = true), 100));
  $("#overlay").addEventListener("mousedown", (e) => {
    if (e.target === e.currentTarget) dismissSheet();
  });
  window.addEventListener("resize", () => {
    if (!st.review) return;
    renderList();
    renderStrip();
  });
  $("#peek").addEventListener("mouseleave", () => hidePeek());
  wireSplits();
  api.events((event, data) => {
    if (event === "progress" && !st.review) {
      st.status = (data as { status: string }).status;
      renderAll();
    } else if (event === "progress") {
      st.notice = (data as { status: string }).status;
      renderHints();
    } else if (event === "changed" && (data as { version: number }).version !== st.version) {
      st.notice = "";
      refresh();
    }
  });
}

async function boot(): Promise<void> {
  wire();
  renderAll();
  applySize(Number(localStorage.getItem("ordo.size")) || 1);
  loadThemes(api).then(() => applyTheme(localStorage.getItem("ordo.look") ?? "auto"));
  await refresh();
}

boot();
