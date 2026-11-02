// src/api.ts
var Api = class {
  token;
  rev;
  constructor(token2, rev2) {
    this.token = token2;
    this.rev = rev2;
  }
  url(path, params = {}) {
    const q = new URLSearchParams({ rev: this.rev });
    for (const [k, v] of Object.entries(params)) q.set(k, String(v));
    return `${path}?${q}`;
  }
  async get(path, params = {}) {
    const res = await fetch(this.url(path, params), { headers: { "X-Ordo-Token": this.token } });
    return answer(res);
  }
  review() {
    return this.get("/api/review");
  }
  file(path) {
    return this.get("/api/file", { path });
  }
  item(i) {
    return this.get("/api/item", { i });
  }
  themes() {
    return this.get("/api/themes");
  }
  async act(body) {
    const res = await fetch(this.url("/api/act"), {
      method: "POST",
      headers: { "X-Ordo-Token": this.token, "Content-Type": "application/json" },
      body: JSON.stringify(body)
    });
    return answer(res);
  }
  events(on) {
    const es = new EventSource(this.url("/api/events", { t: this.token }));
    for (const name of ["progress", "changed"]) {
      es.addEventListener(name, (e) => on(name, JSON.parse(e.data)));
    }
    return es;
  }
};
async function answer(res) {
  const text = await res.text();
  let body = null;
  try {
    body = JSON.parse(text);
  } catch {
  }
  if (!res.ok) throw new Error(body?.error ?? `${res.status} ${res.statusText}`);
  if (body === null) throw new Error("the server sent something that is not JSON");
  return body;
}

// src/dom.ts
function h(tag, attrs = null, ...children) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs ?? {})) {
    if (v === null || v === void 0 || v === false) continue;
    if (k.startsWith("on") && typeof v === "function") {
      el.addEventListener(k.slice(2), v);
    } else if (k === "class") {
      el.className = String(v);
    } else if (k === "dataset") {
      Object.assign(el.dataset, v);
    } else {
      el.setAttribute(k, v === true ? "" : String(v));
    }
  }
  append(el, children);
  return el;
}
function append(el, children) {
  for (const c of children) {
    if (c === null || c === void 0 || c === false) continue;
    if (Array.isArray(c)) append(el, c);
    else el.appendChild(typeof c === "object" ? c : document.createTextNode(String(c)));
  }
}
function $(sel) {
  const el = document.querySelector(sel);
  if (!el) throw new Error(`missing ${sel}`);
  return el;
}
function replace(parent, ...children) {
  parent.replaceChildren();
  append(parent, children);
}

// src/keys.ts
var NAMED = {
  " ": "Space",
  Escape: "Esc",
  Enter: "Enter",
  Backspace: "Backspace",
  Tab: "Tab",
  ArrowUp: "Up",
  ArrowDown: "Down",
  ArrowLeft: "Left",
  ArrowRight: "Right",
  Home: "Home",
  End: "End",
  PageUp: "PageUp",
  PageDown: "PageDown"
};
function keyLabel(e) {
  if (["Shift", "Control", "Alt", "Meta", "CapsLock", "Dead"].includes(e.key)) return null;
  if (e.metaKey) return null;
  const named = NAMED[e.key] ?? (/^F\d{1,2}$/.test(e.key) ? e.key : null);
  const char = !named && [...e.key].length === 1 ? e.key : null;
  if (!named && !char) return null;
  let prefix = "";
  if (e.ctrlKey) prefix += "C-";
  if (e.shiftKey && named && named !== "Space") prefix += "S-";
  if (e.altKey) prefix += "A-";
  return prefix + (named ?? char);
}
var Keymap = class {
  binds;
  constructor(binds) {
    this.binds = binds;
  }
  /** The terminal's rule: a key that opens a chord waits for the next one. */
  resolve(pending, key) {
    const find = (pre) => this.binds.find((b2) => b2.prefix === pre && b2.key === key);
    if (pending !== null) {
      const b2 = find(pending);
      return b2 ? { act: b2.action } : { miss: true };
    }
    if (this.binds.some((b2) => b2.prefix === key)) return { pending: true };
    const b = find(null);
    return b ? { act: b.action } : { miss: true };
  }
  /** What `key` does on its own, even when it also opens a chord. */
  bare(key) {
    return this.binds.find((b) => b.prefix === null && b.key === key)?.action ?? null;
  }
  /** Every chord bound to `action`, as `?` lists them. */
  keysFor(action) {
    return this.binds.filter((b) => b.action === action).map((b) => b.prefix ? `${b.prefix}${b.prefix.length > 1 || b.key.length > 1 ? " " : ""}${b.key}` : b.key);
  }
};

// src/layout.ts
function centerScroll(rowTop, rowHeight2, viewHeight, contentHeight) {
  const want = rowTop + rowHeight2 / 2 - viewHeight / 2;
  const max = Math.max(0, contentHeight - viewHeight);
  return Math.round(Math.min(max, Math.max(0, want)));
}
function tableLimit(total, maximised) {
  return maximised ? total : Math.min(total, 3);
}
function codeRows(f) {
  const hunks = [...f.hunks].sort((a, b) => a.new[0] - b.new[0] || a.old[0] - b.old[0]);
  const rows = [];
  const startingAt = /* @__PURE__ */ new Map();
  for (const h2 of hunks) startingAt.set(h2.new[0], [...startingAt.get(h2.new[0]) ?? [], h2]);
  const added = /* @__PURE__ */ new Map();
  for (const h2 of hunks) {
    for (let n = h2.new[0]; n <= h2.new[1]; n++) {
      added.set(n, { hunk: h2.i, marks: h2.added[n - h2.new[0]] ?? null });
    }
  }
  const removedBefore = (n) => {
    for (const h2 of startingAt.get(n) ?? []) {
      for (let o = h2.old[0]; o <= h2.old[1]; o++) {
        rows.push({
          kind: "del",
          o,
          text: f.old[o - 1] ?? "",
          spans: f.old_spans?.[o - 1] ?? null,
          hunk: h2.i,
          marks: h2.removed[o - h2.old[0]] ?? null
        });
      }
    }
  };
  for (let n = 1; n <= f.new.length; n++) {
    removedBefore(n);
    const a = added.get(n);
    rows.push({
      kind: a ? "add" : "ctx",
      n,
      text: f.new[n - 1],
      spans: f.new_spans?.[n - 1] ?? null,
      hunk: a?.hunk ?? null,
      marks: a?.marks ?? null
    });
  }
  removedBefore(f.new.length + 1);
  return rows;
}
function firstRowOf(rows, hunk) {
  const k = rows.findIndex((r) => r.hunk === hunk);
  return k < 0 ? 0 : k;
}
function runs(text, spans, marks) {
  const chars = [...text];
  const role = new Array(chars.length).fill(null);
  if (spans) {
    let p = 0;
    for (const [t, r] of spans) {
      for (let k = 0; k < [...t].length && p < chars.length; k++) role[p++] = r;
    }
  }
  const changed = new Array(chars.length).fill(false);
  for (const [a, b] of marks ?? []) for (let k = a; k < b && k < chars.length; k++) changed[k] = true;
  const out = [];
  for (let k = 0; k < chars.length; k++) {
    const last = out[out.length - 1];
    if (last && last.role === role[k] && last.changed === changed[k]) last.text += chars[k];
    else out.push({ text: chars[k], role: role[k], changed: changed[k] });
  }
  return out;
}
function wordClass(c) {
  if (/\s/u.test(c)) return 0;
  return /[\p{L}\p{N}_]/u.test(c) ? 1 : 2;
}
function flatten(lines) {
  const chars = [];
  const starts = [];
  lines.forEach((l, k) => {
    starts.push(chars.length);
    chars.push(...l);
    if (k + 1 < lines.length) chars.push("\n");
  });
  return { chars, starts };
}
function caretAt(lines, starts, i) {
  let row = 0;
  while (row + 1 < starts.length && starts[row + 1] <= i) row++;
  return { row, col: Math.min(i - starts[row], [...lines[row]].length) };
}
function wordMove(lines, at, kind) {
  const { chars, starts } = flatten(lines);
  if (!chars.length) return { row: 0, col: 0 };
  let i = Math.min((starts[at.row] ?? 0) + at.col, chars.length - 1);
  const cls = (k) => wordClass(chars[k]);
  if (kind === "next") {
    const c2 = cls(i);
    if (c2 !== 0) while (i + 1 < chars.length && cls(i + 1) === c2) i++;
    while (i + 1 < chars.length && cls(i + 1) === 0) i++;
    return caretAt(lines, starts, Math.min(i + 1, chars.length - 1));
  }
  if (kind === "prev") {
    if (i === 0) return { row: 0, col: 0 };
    i--;
    while (i > 0 && cls(i) === 0) i--;
    const c2 = cls(i);
    while (i > 0 && cls(i - 1) === c2) i--;
    return caretAt(lines, starts, i);
  }
  if (i + 1 >= chars.length) return caretAt(lines, starts, i);
  i++;
  while (i < chars.length && cls(i) === 0) i++;
  if (i >= chars.length) return caretAt(lines, starts, chars.length - 1);
  const c = cls(i);
  while (i + 1 < chars.length && cls(i + 1) === c) i++;
  return caretAt(lines, starts, i);
}

// src/palette.ts
function parseCommand(line) {
  const t = line.trim().replace(/^:/, "").trimStart();
  const sp = t.search(/\s/);
  return sp < 0 ? { name: t, arg: "" } : { name: t.slice(0, sp), arg: t.slice(sp + 1).trim() };
}
var CLIENT_COMMANDS = [
  { name: "theme", args: "<name>", help: "switch the colour theme (auto follows the system)" },
  { name: "size", args: "<small|normal|large|larger|factor>", help: "make the text and rows bigger or smaller, for this browser" }
];
function complete(input, commands, args) {
  const t = input.replace(/^:/, "");
  const sp = t.indexOf(" ");
  if (sp < 0) {
    return commands.map((c) => c.name).filter((n) => n.startsWith(t));
  }
  const name = t.slice(0, sp);
  const typed = t.slice(sp + 1);
  return (args[name] ?? []).filter((a) => a.startsWith(typed)).map((a) => `${name} ${a}`);
}

// src/theme.ts
var themes = {};
function cssVar(role) {
  if (role.startsWith("syntax-")) return `--syn-${role.slice(7)}`;
  const own = { border: "--line", "border-focus": "--focus" };
  if (own[role]) return own[role];
  return ["fg", "dim", "accent", "reviewed"].includes(role) || role.endsWith("-bg") ? `--${role}` : null;
}
function baseOf(theme) {
  const fg = theme["fg"];
  if (!fg || !/^#[0-9a-f]{6}$/i.test(fg)) return null;
  const n = parseInt(fg.slice(1), 16);
  const lum = (0.299 * (n >> 16) + 0.587 * (n >> 8 & 255) + 0.114 * (n & 255)) / 255;
  return lum > 0.5 ? "dark" : "light";
}
async function loadThemes(api2) {
  try {
    const t = await api2.themes();
    themes = t.themes;
    if (!localStorage.getItem("ordo.theme") && t.chosen && t.chosen !== "terminal") {
      localStorage.setItem("ordo.theme", t.chosen);
    }
  } catch {
    themes = {};
  }
}
function themeNames() {
  return Object.keys(themes).filter((n) => n !== "dark" && n !== "light");
}
function applyTheme(name) {
  const root = document.documentElement;
  for (const p of [...root.style]) if (p.startsWith("--") && p !== "--ui") root.style.removeProperty(p);
  root.removeAttribute("data-base");
  const t = themes[name];
  if (!t || name === "auto") return;
  const base = baseOf(t);
  if (base) root.dataset.base = base;
  for (const [role, value] of Object.entries(t)) {
    const v = cssVar(role);
    if (v && value) root.style.setProperty(v, value);
  }
}

// src/main.ts
var PANES = ["list", "code", "why"];
var LEVELS = ["verdict", "warn", "note"];
var SINCE = {
  new: ["\u2726", "new since you last looked"],
  changed: ["\u25D0", "changed since you last looked"],
  moved: ["\u21C5", "unchanged, but it reads in a different place now \u2014 its dependencies moved"]
};
var CHORD_TIMEOUT = 500;
var rev = new URLSearchParams(location.search).get("rev") ?? "HEAD";
var token = new URLSearchParams(location.hash.slice(1)).get("t") ?? sessionStorage.getItem("ordo.token") ?? "";
sessionStorage.setItem("ordo.token", token);
if (location.hash) history.replaceState(null, "", location.pathname + location.search);
var api = new Api(token, rev);
var st = {
  review: null,
  status: "starting\u2026",
  version: -1,
  keymap: new Keymap([]),
  sel: -1,
  focus: "list",
  zoom: false,
  file: null,
  rows: [],
  files: /* @__PURE__ */ new Map(),
  facts: null,
  cursor: 0,
  from: null,
  jumps: [],
  pending: null,
  pendingTimer: 0,
  search: null,
  sinceOnly: false,
  empty: false,
  col: null,
  whyLinks: [],
  whyCur: -1,
  notice: ""
};
var refreshSeq = 0;
var selectSeq = 0;
var filesGen = 0;
async function refresh() {
  const seq = ++refreshSeq;
  let slot;
  try {
    slot = await api.review();
  } catch (e) {
    if (seq !== refreshSeq) return;
    st.status = String(e.message);
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
  const moved = !!prev && prev.generation !== slot.generation;
  const remap = (i) => {
    if (!prev || !moved) return i;
    const id = identity(prev.items[i]);
    return id ? slot.items.findIndex((it) => identity(it) === id) : -1;
  };
  let sel = st.sel >= 0 ? remap(st.sel) : -1;
  if (sel < 0) sel = prev ? clampToView(Math.min(st.sel, slot.items.length - 1)) : slot.sel;
  if (moved) {
    st.jumps = st.jumps.map((j) => ({ ...j, sel: remap(j.sel) })).filter((j) => j.sel >= 0);
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
}
function identity(it) {
  return it ? `${it.path}\0${it.new[0]}\0${it.rationale}` : "";
}
function clampToView(i) {
  const v = st.review.view;
  return v.includes(i) ? i : v[0] ?? 0;
}
async function fileOf(path) {
  const hit = st.files.get(path);
  if (hit) return hit;
  const gen = filesGen;
  const f = await api.file(path);
  if (gen === filesGen) st.files.set(path, f);
  return f;
}
var factsFor = -1;
async function loadFacts(i) {
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
  }
}
async function select(i, opts = {}) {
  const r = st.review;
  if (!r || i < 0 || i >= r.items.length) return renderAll();
  const seq = ++selectSeq;
  const it = r.items[i];
  let f;
  try {
    f = await fileOf(it.path);
  } catch (e) {
    if (seq === selectSeq) toast(e.message, true);
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
async function act(body) {
  try {
    const out = await api.act({
      sel: st.sel,
      generation: st.review?.generation,
      line: lineOfCursor(),
      from: fromLine(),
      ...body
    });
    if (out.error) toast(out.error, true);
    if (out.notice) toast(out.notice);
    if (out.popup) showSheet(out.popup.title, out.popup.lines.join("\n"));
    if (out.clipboard !== void 0) {
      await navigator.clipboard.writeText(out.clipboard).then(
        () => toast("copied the review prompt to the clipboard"),
        () => showSheet("review prompt", out.clipboard)
      );
    }
    if (out.navigate) {
      const q = new URLSearchParams({ rev: out.navigate });
      location.href = `/?${q}#t=${token}`;
    }
    if (out.version > st.version) await refresh();
    if (out.sel !== void 0 && out.sel !== st.sel) await select(out.sel);
    renderConfig();
    return out;
  } catch (e) {
    toast(e.message, true);
    refresh();
    return null;
  }
}
function lineOfCursor() {
  const row = st.rows[st.cursor];
  if (!row) return void 0;
  if (row.kind !== "del") return row.n - 1;
  const next = st.rows.slice(st.cursor).find((r) => r.kind !== "del");
  return next ? next.n - 1 : void 0;
}
function fromLine() {
  if (st.from === null) return void 0;
  const row = st.rows[st.from];
  return row && row.kind !== "del" ? row.n - 1 : void 0;
}
function shownRows() {
  const r = st.review;
  if (!st.sinceOnly) return r.rows;
  const keep = (i) => r.items[i].delta !== null;
  const out = [];
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
function stops() {
  return shownRows().flatMap(
    (row) => row.kind === "item" ? [row.i] : row.kind === "folded" ? [row.items.includes(st.sel) ? st.sel : row.items[0]] : []
  );
}
function step(by) {
  const s = stops();
  if (!s.length) return;
  const at = s.indexOf(st.sel);
  const to = at < 0 ? 0 : Math.max(0, Math.min(s.length - 1, at + by));
  select(s[to]);
}
function rowClass(items, selected) {
  const r = st.review;
  const its = items.map((i) => r.items[i]);
  const reviewed = its.every((it) => it.reviewed);
  const level = LEVELS.find((l) => its.some((it) => it.level === l));
  const quiet = reviewed || its.every((it) => it.noise);
  return ["r", reviewed && "reviewed", level && `lv-${level}`, selected && "sel", quiet && "quiet"].filter(Boolean).join(" ");
}
function renderList() {
  const pane = $("#list");
  const r = st.review;
  replace(
    pane.querySelector(".head"),
    paneHead(1, "list", r.mode === "ledger" ? "by symbol" : "by hunk"),
    st.sinceOnly && h("span", null, "\xB7 since last look"),
    r.path_filter && h("span", null, `\xB7 ${r.path_filter}`),
    h("span", { class: "grow" }),
    zoomButton("list")
  );
  const body = pane.querySelector(".body");
  const rows = h("div", { class: "rows" });
  let selEl = null;
  for (const row of shownRows()) {
    if (row.kind === "file") {
      rows.append(h("div", { class: "fileband" }, h("span", null, row.path), h("span", null, `${row.n} hunk${row.n === 1 ? "" : "s"}`)));
    } else if (row.kind === "gap") {
      rows.append(h("div", { class: "gap" }));
    } else {
      const items = row.kind === "item" ? [row.i] : row.items;
      const first = r.items[items[0]];
      const selected = items.includes(st.sel);
      const text = row.kind === "folded" ? `${items.length} hunk${items.length === 1 ? "" : "s"}, folded` : first.rationale;
      const sym = row.kind === "folded" || row.opens ? first.symbol ?? "" : "";
      const since = first.delta ? SINCE[first.delta][0] : "";
      const el = h(
        "div",
        {
          class: rowClass(items, selected),
          title: first.rationale,
          onclick: () => {
            st.focus = "list";
            select(items.includes(st.sel) ? st.sel : items[0]);
          }
        },
        h("span", { class: "sign" }, since),
        h("span", { class: "sym" }, sym),
        h("span", { class: "txt" }, text),
        h("span", { class: "num" }, first.new[0])
      );
      if (selected) selEl = el;
      rows.append(el);
    }
  }
  replace(body, rows);
  if (selEl) {
    const el = selEl;
    body.scrollTop = centerScroll(el.offsetTop, el.offsetHeight, body.clientHeight, rows.offsetHeight);
  }
}
function rowHeight() {
  return document.querySelector("#code .l")?.offsetHeight || 26;
}
function scrollCodeTo(row, force) {
  if (!force) return;
  const body = $("#code .body");
  const el = body.querySelector(`.l[data-k="${Math.max(0, row - 3)}"]`);
  body.scrollTop = el ? el.offsetTop : 0;
}
function followCursor() {
  const body = $("#code .body");
  const rh = rowHeight();
  const el = body.querySelector(".l.cur");
  const top = el ? el.offsetTop : st.cursor * rh;
  if (top < body.scrollTop) body.scrollTop = top;
  else if (top + rh > body.scrollTop + body.clientHeight) body.scrollTop = top + rh - body.clientHeight;
}
function lineEl(row, k, mine, findings, uses, commented) {
  const search = st.search;
  const caret = k === st.cursor ? caretCol(row.text) : -1;
  let at = 0;
  const parts = runs(row.text, row.spans, row.marks).flatMap((run2) => {
    const piece = (text, extra) => h(run2.changed ? "mark" : "span", { class: [run2.role && `s-${run2.role}`, extra].filter(Boolean).join(" ") || null }, text);
    const cs = [...run2.text];
    const i = caret - at;
    at += cs.length;
    if (i < 0 || i >= cs.length) return [piece(run2.text)];
    const before = cs.slice(0, i).join("");
    const after = cs.slice(i + 1).join("");
    return [before && piece(before), piece(cs[i], "caret"), after && piece(after)].filter((x) => !!x);
  });
  const content = search && search.hits.includes(k) ? highlightHits(row.text, search, k) : parts;
  const n = row.kind === "del" ? "" : row.n;
  const cls = [
    "l",
    row.kind,
    row.hunk !== null && !mine && "other",
    k === st.cursor && "cur",
    st.from !== null && k >= Math.min(st.from, st.cursor) && k <= Math.max(st.from, st.cursor) && "picked",
    row.kind !== "del" && commented.has(row.n) && "commented"
  ].filter(Boolean).join(" ");
  const out = [
    h(
      "div",
      {
        class: cls,
        dataset: { k: String(k) },
        onclick: (e) => {
          st.cursor = k;
          st.col = caretColumn(e, row.text);
          st.focus = "code";
          if (row.hunk !== null && row.hunk !== st.sel) select(row.hunk, { keepCursor: true, focus: "code" });
          else renderCode();
        }
      },
      h("span", { class: "g" }, row.kind !== "del" && uses.has(row.n) ? "\u25B8" : ""),
      h("span", { class: "ln" }, n),
      h("span", { class: "op" }, row.kind === "add" ? "+" : row.kind === "del" ? "-" : " "),
      h("span", null, content)
    )
  ];
  if (row.kind !== "del") {
    for (const f of findings.get(row.n) ?? []) {
      out.push(h("div", { class: `fl ${f.level}` }, h("b", null, `${f.level} ${f.name}`), f.message.split("\n")[0]));
    }
  }
  return out;
}
function highlightHits(text, s, k) {
  const out = [];
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
function renderCode() {
  const pane = $("#code");
  const r = st.review;
  const it = r.items[st.sel];
  const head = pane.querySelector(".head");
  replace(
    head,
    paneHead(2, "code"),
    it && h("span", { class: "path" }, it.path),
    it && h("span", null, `L${it.new[0]}`),
    st.search && h("span", null, `/${st.search.query} ${st.search.hits.length ? `${st.search.k + 1}/${st.search.hits.length}` : "no match"}`),
    st.from !== null && h("span", null, "selecting lines \xB7 c comments them"),
    h("span", { class: "grow" }),
    zoomButton("code")
  );
  const body = pane.querySelector(".body");
  if (!st.file || !it) return replace(body);
  const findings = /* @__PURE__ */ new Map();
  for (const f of it.findings) {
    if (f.row == null) continue;
    findings.set(f.row, [...findings.get(f.row) ?? [], f]);
  }
  const uses = new Set(it.uses_at.flatMap((u) => u.rows));
  const commented = new Set(st.file.comments.flatMap((c) => range(c.start, c.end)));
  const code = h("div", { class: "code" });
  st.rows.forEach((row, k) => {
    for (const el of lineEl(row, k, row.hunk === st.sel, findings, uses, commented)) code.append(el);
    if (row.kind !== "del") {
      for (const c of st.file.comments.filter((c2) => c2.end === row.n)) {
        code.append(h("div", { class: "cmt" }, `\u270E ${c.text}${c.stale ? " (lines changed since)" : ""}`));
      }
    }
  });
  const top = body.scrollTop;
  replace(body, code);
  body.scrollTop = top;
}
function caretColumn(e, text) {
  const doc = document;
  const pos = doc.caretPositionFromPoint?.(e.clientX, e.clientY);
  const line = e.currentTarget.lastElementChild;
  if (!pos || !line || !line.contains(pos.offsetNode)) return null;
  const walker = document.createTreeWalker(line, NodeFilter.SHOW_TEXT);
  let at = 0;
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    if (n === pos.offsetNode) return Math.min(at + [...(n.textContent ?? "").slice(0, pos.offset)].length, [...text].length);
    at += [...n.textContent ?? ""].length;
  }
  return null;
}
function caretCol(text) {
  if (st.col === null) return -1;
  return Math.min(st.col, Math.max(0, [...text].length - 1));
}
function firstWordCol(text) {
  const m = /[A-Za-z_]/.exec(text);
  return m ? [...text.slice(0, m.index)].length : 0;
}
function openEditor() {
  const r = st.review;
  const it = r.items[st.sel];
  if (!it) return;
  const row = st.rows[st.cursor];
  const line = row && row.kind !== "del" ? row.n : it.new[0];
  if (r.editor === "server") {
    act({ op: "open-editor" });
    return;
  }
  const segments = (p) => p.split("/").map(encodeURIComponent).join("/");
  const url = (r.editor || "vscode://file/{root}/{path}:{line}").replaceAll("{root}", segments(r.root).replace(/^\//, "")).replaceAll("{path}", segments(it.path)).replaceAll("{line}", String(line));
  window.open(url, "_self");
}
function range(a, b) {
  return Array.from({ length: Math.max(0, b - a + 1) }, (_, k) => a + k);
}
function moveCursor(by) {
  cursorToRow(Math.max(0, Math.min(st.rows.length - 1, st.cursor + by)));
}
function cursorToRow(k) {
  st.cursor = k;
  const hunk = st.rows[st.cursor]?.hunk;
  if (hunk !== null && hunk !== void 0 && hunk !== st.sel) {
    select(hunk, { keepCursor: true }).then(followCursor);
    return;
  }
  renderCode();
  followCursor();
}
function cursorTo(pred, forward) {
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
function edgeRows(targets, limit) {
  const r = st.review;
  return targets.slice(0, limit).map(({ target, label }) => {
    if (target === null || !r.items[target]) {
      return h("tr", { class: "out" }, h("td", null, label), h("td", { class: "loc" }, ""), h("td", null, "not part of this review"));
    }
    const t = r.items[target];
    const link = st.whyLinks.length;
    st.whyLinks.push(target);
    return h(
      "tr",
      {
        class: `link${st.whyCur === link ? " cur" : ""}`,
        title: "go there (Enter); back with C-o",
        onclick: () => jumpTo(target)
      },
      h("td", null, t.symbols[0] ?? t.enclosing ?? t.symbol ?? label),
      h("td", { class: "loc" }, `${basename(t.path)}:${t.new[0]}`),
      h("td", null, t.rationale)
    );
  });
}
function basename(p) {
  return p.slice(p.lastIndexOf("/") + 1);
}
function table(title, hint, targets, empty) {
  const limit = tableLimit(targets.length, st.zoom && st.focus === "why");
  const more = targets.length - limit;
  return [
    h("h3", null, title, hint && h("small", null, hint)),
    targets.length ? h("table", null, h("tbody", null, edgeRows(targets, limit))) : h("p", { class: "muted" }, empty),
    more > 0 && h(
      "button",
      {
        class: "more",
        onclick: () => {
          st.focus = "why";
          st.zoom = true;
          renderAll();
        }
      },
      `${more} more \u2014 maximise the pane to see them`
    )
  ];
}
function rulesChecked(r) {
  const parts = [];
  parts.push(r.rules.catalog ? "the construct catalog" : "no catalog (switched off)");
  if (r.rules.own) parts.push(`${r.rules.own} rule${r.rules.own === 1 ? "" : "s"} of your own`);
  if (r.rules.includes.length) parts.push(`rulesets ${r.rules.includes.join(", ")}`);
  return parts.join(", ");
}
function renderWhy() {
  const pane = $("#why");
  const r = st.review;
  const it = r.items[st.sel];
  replace(pane.querySelector(".head"), paneHead(3, "why"), it && h("span", { class: "path" }, it.symbols[0] ?? it.enclosing ?? ""), h("span", { class: "grow" }), zoomButton("why"));
  const body = pane.querySelector(".body");
  if (!it) return replace(body);
  st.whyLinks = [];
  const facts = st.facts && st.facts.i === st.sel ? st.facts : null;
  const needs = it.edges.filter((e) => e.needs);
  const neededBy = it.edges.filter((e) => !e.needs);
  const findings = [...it.findings].sort((a, b) => LEVELS.indexOf(a.level) - LEVELS.indexOf(b.level));
  const top = body.scrollTop;
  replace(
    body,
    h(
      "div",
      { class: "why" },
      it.note && [h("h3", null, "your note"), h("p", { class: "warnline" }, it.note)],
      !!facts?.comments.length && [
        h("h3", null, "your comments"),
        facts.comments.map((c) => h("p", { class: "warnline" }, `L${c.start}${c.end > c.start ? `\u2013${c.end}` : ""} ${c.text}${c.stale ? " (lines changed since)" : ""}`))
      ],
      !!facts?.out_of_order.length && [
        h("h3", null, "order"),
        h("p", { class: "warnline" }, "marked reviewed, but it depends on hunks not reviewed yet:"),
        h("table", null, h("tbody", null, edgeRows(facts.out_of_order.map((t) => ({ target: t, label: "" })), facts.out_of_order.length)))
      ],
      h(
        "div",
        { class: "what" },
        h("h3", null, "what"),
        it.what.length ? it.what.map((w) => h("p", null, w)) : h("p", { class: "muted" }, "ordo found nothing to say about this hunk"),
        it.details.map((d) => h("p", { class: "detail" }, d))
      ),
      it.grouped_by && [h("h3", null, "grouped by", h("small", null, "why it sits with its neighbours")), h("p", null, it.grouped_by)],
      table("needs", "what this hunk uses", needs, "nothing in this change"),
      table("needed by", "what uses this hunk", neededBy, "nothing in this change"),
      facts && table(
        "if rejected",
        "everything that loses its footing, in reading order",
        facts.cascade.map((t) => ({ target: t, label: "" })),
        "rejecting this hunk strands nothing"
      ),
      it.executed && [
        h("h3", null, "coverage"),
        h(
          "p",
          { class: it.executed[0] === it.executed[1] ? "" : "warnline" },
          it.executed[0] === it.executed[1] ? `all ${it.executed[1]} executable lines here are covered by tests` : `${it.executed[1] - it.executed[0]} of ${it.executed[1]} executable lines here are executed by no test`
        )
      ],
      it.notes.length > 0 && [h("h3", null, "notes"), it.notes.map((n) => h("p", null, n))],
      it.wave !== null && [
        h("h3", null, "wave"),
        h("p", null, `wave ${it.wave}${r.wave_intents[String(it.wave)] ? ` \xB7 asked: ${r.wave_intents[String(it.wave)]}` : ""}`)
      ],
      h("h3", null, "findings", h("small", null, findings.length ? "the hardest-pressing first" : "")),
      findings.length ? findings.map(
        (f) => h(
          "div",
          { class: "finding" },
          h(
            "div",
            { class: "h" },
            h("span", { class: `chip ${f.level}` }, f.level),
            h("b", null, f.name),
            h("span", { class: "src" }, f.source + (f.original_level ? `, ${f.original_level} in its rule, moved by rules.toml` : "")),
            f.row != null && h("button", { class: "at", onclick: () => cursorToLine(f.row) }, `L${f.row}`)
          ),
          h("p", null, f.message)
        )
      ) : h("p", { class: "muted" }, `No findings. Checked: ${rulesChecked(r)}.`)
    )
  );
  body.scrollTop = top;
}
function cursorToLine(n) {
  const k = st.rows.findIndex((row) => row.kind !== "del" && row.n === n);
  if (k < 0) return;
  st.cursor = k;
  st.focus = "code";
  renderAll();
  followCursor();
}
function jumpTo(target) {
  st.jumps.push({ sel: st.sel, cursor: st.cursor });
  select(target, { focus: "code" });
}
function jumpBack() {
  const j = st.jumps.pop();
  if (!j) return;
  select(j.sel, { focus: "code" }).then(() => {
    st.cursor = Math.min(j.cursor, st.rows.length - 1);
    renderCode();
    followCursor();
  });
}
function renderBand() {
  const band = $("#band");
  const r = st.review;
  if (!r) {
    return replace(
      band,
      h("span", { class: "mark" }, "ordo"),
      h("span", { class: "rev" }, rev),
      h("span", { class: "subject" }, st.status),
      st.empty && h("button", { class: "sig", onclick: () => act({ op: "reload" }) }, "read again")
    );
  }
  const it = r.items[st.sel];
  const since = it?.delta ? SINCE[it.delta] : null;
  const sinceCount = r.delta.new + r.delta.changed + r.delta.moved;
  const churn = it ? r.file_churn[it.path] : void 0;
  const lc = st.facts?.i === st.sel ? st.facts.line_churn : null;
  const pct = r.coverage.total ? 100 * r.coverage.done / r.coverage.total : 0;
  replace(
    band,
    h("span", { class: "mark" }, "ordo", h("span", null, ".")),
    h("span", { class: "rev" }, r.rev),
    h("span", { class: "subject", title: r.subject ?? "" }, r.subject ?? (r.uncommitted ? "uncommitted work" : "")),
    r.stale && h("button", { class: "sig stale", onclick: () => act({ op: "reload" }) }, r.stale),
    r.watch !== "off" && h(
      "span",
      { class: "sig", title: r.watch === "auto" ? "--watch=auto: read again once the working tree settles and you pause" : "--watch=hint: says when the working tree has moved on" },
      r.watch === "auto" ? "watching, reloads" : "watching"
    ),
    since && h("span", { class: "sig", title: since[1] }, h("span", { class: "glyph" }, since[0]), since[1].split(",")[0]),
    !r.delta.first_run && sinceCount > 0 && h(
      "button",
      {
        class: "sig",
        "aria-pressed": st.sinceOnly ? "true" : "false",
        title: `${r.delta.new} new, ${r.delta.changed} changed, ${r.delta.moved} moved, ${r.delta.gone} gone \u2014 show only these`,
        onclick: () => {
          st.sinceOnly = !st.sinceOnly;
          renderAll();
        }
      },
      h("span", { class: "glyph" }, "\u25D0"),
      h("b", null, sinceCount),
      "since last look"
    ),
    churn !== void 0 && churn > 0 && h("span", { class: "sig", title: "commits that touched this file in the last 6 months" }, "\u27F3", h("b", null, churn), "/6mo"),
    lc && h(
      "span",
      { class: "sig", title: "how often these lines changed before, and who touched them last" },
      "lines \u27F3",
      h("b", null, `${lc.commits}${lc.capped ? "+" : ""}`),
      lc.last && `\xB7 ${lc.last.author} ${lc.last.date}`
    ),
    h("span", { class: "sig", title: "hunks reviewed" }, h("span", { class: "meter" }, h("i", { style: `width:${pct}%` })), h("b", null, r.coverage.done), `/${r.coverage.total}`),
    h("span", { class: "sig", title: "def\u2192use links with both ends reviewed" }, h("b", null, r.coverage.both), `/${r.coverage.edges} edges`),
    LEVELS.filter((l) => r.levels[l]).map(
      (l) => h("button", { class: `chip ${l}`, style: "border:0;cursor:pointer", title: `next ${l}`, onclick: () => act({ op: "action", action: "finding-next" }) }, `${r.levels[l]} ${l}`)
    )
  );
}
var HINTS = {
  list: [["next", "move"], ["toggle-reviewed", "reviewed"], ["finding-next", "finding"], ["fold-toggle", "fold"], ["command", "command"], ["help", "all keys"]],
  code: [["next", "line"], ["search", "search"], ["mark-next", "next use"], ["select-lines", "select"], ["comment", "comment"], ["jump-back", "back"]],
  why: [["next", "link"], ["jump-to-edge", "go there"], ["jump-back", "back"], ["zoom", "maximise"], ["help", "all keys"]]
};
function renderHints() {
  const items = HINTS[st.focus].map(([action, label]) => {
    const k = st.keymap.keysFor(action)[0];
    return k ? h("span", null, h("kbd", null, k), label) : null;
  });
  replace($("#hints"), items, st.notice && h("span", { class: "notice" }, st.notice));
}
function paneHead(n, title, sub) {
  return [h("span", { class: "n" }, n), h("span", { class: "t" }, title), sub ? h("span", null, sub) : null].filter(Boolean);
}
function zoomButton(p) {
  const on = st.zoom && st.focus === p;
  return h(
    "button",
    {
      title: on ? "restore the panes" : "maximise this pane",
      onclick: () => {
        st.zoom = !on;
        st.focus = p;
        renderAll();
      }
    },
    on ? "\u2921" : "\u2922"
  );
}
function showSheet(title, text) {
  const ov = $("#overlay");
  ov.hidden = false;
  replace(
    ov,
    h(
      "div",
      { class: "sheet", role: "dialog", "aria-label": title },
      h("header", null, title, h("button", { onclick: closeSheet, "aria-label": "close" }, "\u2715")),
      typeof text === "string" ? h("pre", null, text) : h("div", { class: "scroll" }, text)
    )
  );
}
function dismissSheet() {
  if ($("#overlay").dataset.kind === "config") {
    act({ op: "config", do: "close" });
    return true;
  }
  return closeSheet();
}
function closeSheet() {
  const ov = $("#overlay");
  if (ov.hidden) return false;
  ov.hidden = true;
  delete ov.dataset.kind;
  canvas = null;
  replace(ov);
  return true;
}
function showHelp() {
  const r = st.review;
  if (!r) return;
  const by = /* @__PURE__ */ new Map();
  for (const b of r.keys) {
    const k = b.prefix ? `${b.prefix} ${b.key}` : b.key;
    by.set(b.action, [...by.get(b.action) ?? [], k]);
  }
  const rows = [...by].map(([a, ks]) => h("tr", null, h("td", null, ks.join(", ")), h("td", null, a)));
  const cmds = [...r.commands, ...CLIENT_COMMANDS].map((c) => h("tr", null, h("td", null, `:${c.name} ${c.args}`), h("td", null, c.help)));
  showSheet("keys and commands", h("div", { class: "why" }, h("table", null, h("tbody", null, rows)), h("h3", null, "commands"), h("table", null, h("tbody", null, cmds))));
}
var toastTimer = 0;
function toast(msg, err = false) {
  const t = $("#toast");
  t.textContent = msg;
  t.className = `show${err ? " err" : ""}`;
  clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => t.className = "", err ? 4e3 : 2200);
}
function renderAll() {
  document.body.classList.toggle("zoom", st.zoom);
  for (const p of PANES) {
    const el = $(`#${p}`);
    el.classList.toggle("focus", st.focus === p);
    el.classList.toggle("zoomed", st.zoom && st.focus === p);
  }
  renderBand();
  if (!st.review) {
    for (const p of PANES) replace($(`#${p} .body`));
    replace($("#code .body"), h("div", { class: "loading" }, st.status));
    renderHints();
    return;
  }
  renderList();
  renderCode();
  renderWhy();
  renderHints();
}
var SIZES = { small: 0.9, normal: 1, large: 1.15, larger: 1.3 };
function sizeOf(arg) {
  const a = arg.trim();
  if (!a) return 1;
  if (a in SIZES) return SIZES[a];
  const n = a.endsWith("%") ? parseFloat(a) / 100 : parseFloat(a);
  return Number.isFinite(n) && n >= 0.6 && n <= 2.5 ? n : null;
}
function applySize(ui) {
  document.documentElement.style.setProperty("--ui", String(ui));
}
var paletteMode = "command";
var paletteCur = 0;
var palettePicked = false;
function openPalette(mode, text = "") {
  paletteMode = mode;
  const form = $("#palette");
  form.hidden = false;
  $("#palette label").textContent = mode === "command" ? ":" : "/";
  const input = $("#palette-input");
  input.value = text;
  paletteCur = 0;
  palettePicked = false;
  renderPalette();
  input.focus();
}
function closePalette() {
  $("#palette").hidden = true;
  document.activeElement?.blur();
}
function argCandidates() {
  const r = st.review;
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
    filter: [...new Set(r.items.map((i) => i.path.replace(/[^/]*$/, "*")))]
  };
}
function candidates() {
  if (paletteMode !== "command" || !st.review) return [];
  const input = $("#palette-input").value;
  return complete(input, [...st.review.commands, ...CLIENT_COMMANDS], argCandidates()).slice(0, 40);
}
function renderPalette() {
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
          onmousedown: (e) => {
            e.preventDefault();
            $("#palette-input").value = c;
            runPalette();
          }
        },
        h("b", null, c),
        h("span", null, cmd && !c.includes(" ") ? cmd.help : "")
      );
    })
  );
}
async function runPalette() {
  const picked = palettePicked && candidates()[paletteCur];
  const value = picked || $("#palette-input").value;
  closePalette();
  if (paletteMode === "search") return runSearch(value);
  const { name, arg } = parseCommand(value);
  if (!name) return;
  if (name === "theme") {
    applyTheme(arg || "auto");
    localStorage.setItem("ordo.theme", arg || "auto");
    return;
  }
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
function runSearch(query, forward = true) {
  if (!query) {
    st.search = null;
    return renderCode();
  }
  const q = query.toLowerCase();
  const hits = st.rows.flatMap((row, k2) => row.text.toLowerCase().includes(q) ? [k2] : []);
  const after = hits.findIndex((x) => x > st.cursor);
  const at = hits.indexOf(st.cursor);
  const k = forward ? Math.max(0, at >= 0 ? at : after) : Math.max(0, (after < 0 ? hits.length : after) - (at >= 0 ? 2 : 1));
  st.search = { query, hits, k };
  if (hits.length) st.cursor = hits[k];
  st.focus = "code";
  renderAll();
  followCursor();
}
function searchStep(by) {
  const s = st.search;
  if (!s || !s.hits.length) return;
  s.k = (s.k + by + s.hits.length) % s.hits.length;
  st.cursor = s.hits[s.k];
  renderCode();
  followCursor();
}
function page(by) {
  const body = $(`#${st.focus} .body`);
  body.scrollTop += by * body.clientHeight;
}
function run(action) {
  const r = st.review;
  if (!r) return;
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
      if (focus === "list") return void (stops().length && select(stops().at(-1)));
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
      const p = action.slice(6);
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
      if (focus === "why" && st.whyLinks[st.whyCur] !== void 0) jumpTo(st.whyLinks[st.whyCur]);
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
      const c = new Set(st.file?.comments.map((c2) => c2.start) ?? []);
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
      st.col = action === "line-start" ? 0 : action === "line-end" ? last : Math.max(0, Math.min(last, now + (action === "cursor-right" ? 1 : -1)));
      st.focus = "code";
      return renderAll();
    }
    case "word-next":
    case "word-prev":
    case "word-end": {
      const kind = action === "word-next" ? "next" : action === "word-prev" ? "prev" : "end";
      const text = st.rows[st.cursor]?.text ?? "";
      const to = wordMove(
        st.rows.map((r2) => r2.text),
        { row: st.cursor, col: st.col === null ? firstWordCol(text) : caretCol(text) },
        kind
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
      if (focus === "why" && st.whyLinks[st.whyCur] !== void 0) return void previewHunk(st.whyLinks[st.whyCur]);
      act({ op: "hover", col: st.col === null ? firstWordCol(st.rows[st.cursor]?.text ?? "") : caretCol(st.rows[st.cursor]?.text ?? "") });
      return;
    case "open-editor":
      return openEditor();
    case "open-deps":
    case "focus-deps":
      return void openCanvas();
  }
}
function whyStep(by) {
  const n = st.whyLinks.length;
  if (!n) {
    $("#why .body").scrollTop += by * 40;
    return;
  }
  st.whyCur = Math.max(0, Math.min(n - 1, st.whyCur + by));
  renderWhy();
  $("#why tr.cur")?.scrollIntoView({ block: "nearest" });
}
function onKey(e) {
  const t = e.target;
  if (t && (t.tagName === "INPUT" || t.tagName === "SELECT" || t.tagName === "TEXTAREA") && t.id !== "palette-input") return;
  if (t?.tagName === "BUTTON" && (e.key === "Enter" || e.key === " ")) return;
  const paletteOpen = !$("#palette").hidden;
  if (paletteOpen) return paletteKey(e);
  const key = keyLabel(e);
  if (!key) return;
  if (key === "Enter" && canvas) {
    e.preventDefault();
    return void canvasKey("jump-to-edge");
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
    const bare = st.keymap.bare(key);
    if (bare) st.pendingTimer = window.setTimeout(() => (st.pending = null, run(bare)), CHORD_TIMEOUT);
    return;
  }
  if ("act" in res) {
    e.preventDefault();
    if (canvasKey(res.act)) return;
    return run(res.act);
  }
  if (pending) {
    const bare = st.keymap.bare(pending);
    if (bare) run(bare);
    onKey(e);
  }
}
function paletteKey(e) {
  const c = candidates();
  if (e.key === "Escape") {
    e.preventDefault();
    return closePalette();
  }
  if (e.key === "Tab" || e.key === "ArrowDown" || e.key === "ArrowUp") {
    e.preventDefault();
    if (!c.length) return;
    if (e.key === "Tab") {
      $("#palette-input").value = c[paletteCur] + (c[paletteCur].includes(" ") ? "" : " ");
      paletteCur = 0;
    } else {
      paletteCur = (paletteCur + (e.key === "ArrowDown" ? 1 : c.length - 1)) % c.length;
      palettePicked = true;
    }
    return renderPalette();
  }
}
function excerpt(f, hunk, context, max) {
  if (f.missing) return h("p", { class: "muted" }, "no text to show for this file");
  const rows = codeRows(f);
  const ks = rows.flatMap((r, k) => r.hunk === hunk ? [k] : []);
  const from = Math.max(0, (ks[0] ?? 0) - context);
  const to = Math.min(rows.length, Math.min((ks.at(-1) ?? 0) + context + 1, from + max));
  return h(
    "div",
    { class: "code" },
    rows.slice(from, to).map(
      (row) => h(
        "div",
        { class: `l ${row.kind}${row.hunk !== null && row.hunk !== hunk ? " other" : ""}` },
        h("span", { class: "g" }),
        h("span", { class: "ln" }, row.kind === "del" ? "" : row.n),
        h("span", { class: "op" }, row.kind === "add" ? "+" : row.kind === "del" ? "-" : " "),
        h("span", null, runs(row.text, row.spans, row.marks).map((run2) => h(run2.changed ? "mark" : "span", { class: run2.role ? `s-${run2.role}` : null }, run2.text)))
      )
    )
  );
}
function hunkTitle(i) {
  const t = st.review.items[i];
  return `${t.symbols[0] ?? t.enclosing ?? t.symbol ?? basename(t.path)} \xB7 ${basename(t.path)}:${t.new[0]}`;
}
async function previewHunk(i) {
  const t = st.review.items[i];
  const f = await fileOf(t.path);
  showSheet(hunkTitle(i), h("div", null, h("p", { class: "muted" }, t.rationale), h("div", { class: "src" }, excerpt(f, i, 2, 40))));
}
var canvas = null;
async function openCanvas() {
  const r = st.review;
  const it = r.items[st.sel];
  const cards = it.edges.filter((e) => e.target !== null && r.items[e.target]).map((e) => ({ i: e.target, needs: e.needs }));
  if (!cards.length) return toast("nothing in this change needs this hunk, and it needs nothing");
  canvas = { anchor: st.sel, cards, sel: 0 };
  await renderCanvas();
}
async function renderCanvas() {
  if (!canvas) return;
  const c = canvas;
  const files = await Promise.all([c.anchor, ...c.cards.map((k) => k.i)].map((i) => fileOf(st.review.items[i].path)));
  if (canvas !== c) return;
  const card = (k) => {
    const { i } = c.cards[k];
    return h(
      "div",
      { class: `card${k === c.sel ? " cur" : ""}`, onclick: () => goCanvas(k) },
      h("header", null, hunkTitle(i), h("span", { class: "muted" }, st.review.items[i].rationale)),
      excerpt(files[k + 1], i, 1, 14)
    );
  };
  const side = (needs) => h(
    "div",
    { class: "side" },
    h("h3", null, needs ? "needs \u2014 what this hunk uses" : "needed by \u2014 what uses this hunk"),
    c.cards.map((x, k) => x.needs === needs ? card(k) : null)
  );
  showSheet(
    `dependencies of ${hunkTitle(c.anchor)}`,
    h(
      "div",
      { class: "canvas src" },
      h("div", { class: "anchor" }, excerpt(files[0], c.anchor, 1, 16)),
      h("div", { class: "sides" }, side(true), side(false))
    )
  );
  $("#overlay").querySelector(".sheet")?.classList.add("wide");
  $("#overlay .card.cur")?.scrollIntoView({ block: "nearest" });
}
function goCanvas(k) {
  if (!canvas) return;
  const target = canvas.cards[k].i;
  canvas = null;
  closeSheet();
  jumpTo(target);
}
function canvasKey(action) {
  if (!canvas) return false;
  const n = canvas.cards.length;
  if (action === "next" || action === "prev") {
    canvas.sel = (canvas.sel + (action === "next" ? 1 : n - 1)) % n;
    renderCanvas();
  } else if (action === "jump-to-edge") {
    goCanvas(canvas.sel);
  } else if (action === "quit" || action === "open-deps" || action === "focus-deps") {
    canvas = null;
    closeSheet();
  }
  return true;
}
function renderConfig() {
  const cfg = st.review?.config;
  const ov = $("#overlay");
  if (!cfg) {
    if (ov.dataset.kind === "config") closeSheet();
    return;
  }
  const step2 = (body2) => act({ op: "config", ...body2 });
  const field = (f) => {
    let control;
    if (f.kind === "flag") {
      control = h("input", { type: "checkbox", checked: f.value === true, onchange: () => step2({ do: "toggle", row: f.row }) });
    } else if (f.kind === "choice") {
      control = h(
        "button",
        { class: "choice", title: (f.options ?? []).join(" \xB7 "), onclick: () => step2({ do: "toggle", row: f.row }) },
        String(f.value ?? "")
      );
    } else {
      control = h("input", {
        value: String(f.value ?? ""),
        placeholder: "(terminal)",
        onchange: (e) => step2({ do: "edit", row: f.row, text: e.target.value })
      });
    }
    return h("tr", null, h("td", null, f.label), h("td", null, control), h("td", { class: "muted" }, f.help));
  };
  const body = h(
    "div",
    { class: "why config" },
    cfg.sections.map((s) => [h("h3", null, s.title, h("small", null, s.file)), h("table", null, h("tbody", null, s.fields.map(field)))])
  );
  const top = ov.querySelector(".scroll")?.scrollTop ?? 0;
  showSheet(cfg.dirty ? "settings \u2014 changed, not written" : "settings", body);
  ov.dataset.kind = "config";
  const header = ov.querySelector(".sheet > header");
  header.insertBefore(h("button", { class: "write", disabled: !cfg.dirty, onclick: () => step2({ do: "write" }) }, "Write to files"), header.lastChild);
  header.lastElementChild.addEventListener("click", () => step2({ do: "close" }));
  ov.querySelector(".scroll").scrollTop = top;
}
function wireSplits() {
  const panes = $("#panes");
  let saved = null;
  try {
    saved = JSON.parse(localStorage.getItem("ordo.splits") ?? "null");
  } catch {
    localStorage.removeItem("ordo.splits");
  }
  if (saved) {
    panes.style.setProperty("--c0", saved[0]);
    panes.style.setProperty("--c2", saved[1]);
  }
  for (const split of document.querySelectorAll(".split")) {
    split.addEventListener("pointerdown", (e) => {
      split.setPointerCapture(e.pointerId);
      split.classList.add("drag");
      const which = split.dataset.split;
      const move = (ev) => {
        const box = panes.getBoundingClientRect();
        const x = (ev.clientX - box.left) / box.width * 100;
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
function wire() {
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
  const form = $("#palette");
  form.addEventListener("submit", (e) => {
    e.preventDefault();
    runPalette();
  });
  $("#palette-input").addEventListener("input", () => {
    paletteCur = 0;
    palettePicked = false;
    renderPalette();
  });
  $("#palette-input").addEventListener("blur", () => setTimeout(() => form.hidden = true, 100));
  $("#overlay").addEventListener("mousedown", (e) => {
    if (e.target === e.currentTarget) dismissSheet();
  });
  window.addEventListener("resize", () => st.review && renderList());
  wireSplits();
  api.events((event, data) => {
    if (event === "progress" && !st.review) {
      st.status = data.status;
      renderAll();
    } else if (event === "progress") {
      st.notice = data.status;
      renderHints();
    } else if (event === "changed" && data.version !== st.version) {
      st.notice = "";
      refresh();
    }
  });
}
async function boot() {
  wire();
  renderAll();
  applySize(Number(localStorage.getItem("ordo.size")) || 1);
  loadThemes(api).then(() => applyTheme(localStorage.getItem("ordo.theme") ?? "auto"));
  await refresh();
}
boot();
