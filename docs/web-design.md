# Web reviewer — design

**Status:** built. The terminal reviewer it was designed beside has since
been removed; `ordo <rev>` starts this one.
**Problem:** a terminal grid is too tight for what a review needs to show
side by side: a reading-order list, the code, and a why pane whose tables
(what a hunk needs, what needs it, what breaks if it is rejected) get cut to
"and 3 more". The web reviewer is a first-party client of the engine, in the
same binary, reading the same caches.

## Goal / non-goals
- **Goal:** `ordo web <rev> [<glob>...]` — the same revisions, filters,
  rules, marks, notes, comments and waves as `ordo <rev>`, rendered in a
  browser.
- **Goal:** one source of truth for review logic. The Rust side builds a
  ready-to-render view model; the browser renders it and posts mutations.
- **Non-goal:** a hosted service. The server binds `127.0.0.1` and lives as
  long as the command does.
- **Non-goal:** flipping `ordo <rev>` to the web reviewer. That happens in a
  later change, once the web reviewer reaches parity.

## Engine changes that land first
Both clients benefit, so they go in before any web code.

- **Formatting only.** A hunk is formatting when its tree-sitter leaf
  tokens are equal on both sides, ignoring whitespace and a trailing comma
  before a closing bracket, *and* each leaf keeps the same ancestor
  node-kind path. Re-wrapping a signature or joining a comprehension is
  formatting. A python reindent that moves a line into or out of a block is
  not, because that line's ancestry changes. The `split_whitespace`
  comparison this replaces missed `f(a, b)` ↔ an exploded `f(\n a, b\n)`,
  because the token next to the bracket changes.
- **A root swallows its children.** When the hunk adds or removes a whole
  definition, the detail layer does not also list what changed inside it.
  `removes _shroud_outer` no longer comes with `removes axis from
  np.linalg.norm`.

## Shape

```
ordo <rev>
  └─ review layer
       gather (git / GitButler) → ordo::run → items → overlays (SARIF, lcov,
       waves) → derived facts (delta, cascade, grouping, churn)
       persistence: marks, notes, comments, run snapshots (~/.cache/ordo)
  └─ server (tiny_http, feature `web`)
       GET  /                        index.html + app.js + app.css (embedded)
       GET  /api/review?rev=…        view model (JSON)
       GET  /api/file?rev=…&path=…   one file, both sides, highlighted by role
       GET  /api/item?rev=…&i=…      the why pane's costlier facts for one hunk
       POST /api/act?rev=…           mark, fold, reload, a served action, hover,
                                     settings, open-in-editor, a palette command
       GET  /api/events?rev=…        SSE: load progress, reload, marks changed
```

- **Review layer.** The logic in `src/bin/ordo`: gathering, `Item`, marks,
  notes, comments, waves, churn, SARIF/lcov placement, and the derived
  facts. Highlighting produces **role names** (the twelve syntax roles),
  never colours; the browser maps them to CSS variables.
  The server holds one `Session` and `App` per revision and calls the load,
  watch and command functions directly.
- **Transport.** `tiny_http` (synchronous, threads, no async runtime) sits
  behind a default-on `web` feature. Server-Sent Events carry the one-way
  pushes the client needs:
  - load progress;
  - `--watch` reloads;
  - marks or notes changed by another tab or another `ordo`.
- **Revisions.** The URL carries the rev (`/?rev=at`). One process caches
  several loaded revisions, so tabs can hold different ones, and `:e <rev>`
  switches without restarting.
- **Safety.**
  - The server binds `127.0.0.1` only.
  - A random session token is part of the printed URL, and every request
    must carry it.
  - The `Host` header is checked against DNS rebinding.
  - Marks, notes and comments are written atomically with read-merge-write,
    so several tabs and processes never clobber each other.

## Client
- **Build.** Vanilla TypeScript in `web/src`, bundled by esbuild through
  `web/package.json`. The bundle (`web/dist/app.js`, `app.css`) is committed
  and `include_str!`'d, so `cargo install` needs no node. A test fails when
  `dist` is stale relative to `src`.
- **Top band.** It keeps the review-wide signals in one place:
  - the rev and its subject;
  - review progress;
  - finding-level chips;
  - **since**: the selected hunk's change since your last open — `✦` new,
    `◐` changed, `⇅` moved, nothing when unchanged. A review-wide count
    beside it (`◐ 7 since last look`) filters the list to those hunks when
    clicked. The same glyph sits in the list row's sign column.
  - **history**: file churn (`⟳ 9/6mo`), and the line churn of the selected
    hunk (`⟳ 3 · alice 2w`), fetched in the background rather than on a key.
- **Columns.** The list, code and why panes sit side by side, with visible
  separators and draggable widths. Any pane can be maximised. Below
  ~1100px they stack. The dependency canvas opens as an overlay.
- **List.** The selected row stays vertically centred, except near the start
  and end of the list.
- **Code.** Selecting a hunk scrolls the code so that the hunk's *first* row
  sits about three rows below the top. That row is the first deleted line
  when there is one; today a removal hunk shows only its last lines.
- **Why pane.**
  - `what`: the rationale clauses and the detail layer.
  - `grouped by` (was `scope`): why the hunk sits with its neighbours.
  - `needs`, `needed by` and `if rejected` (the cascade) are tables with the
    columns symbol, `file:line` and what it does. They show 3 rows by
    default, and every row when the pane is maximised.
  - Coverage, notes and wave follow.
  - **Findings** are always present. When nothing fired, the section names
    what ran (catalog, `rules.toml`, rulesets), so an empty pane never reads
    as rules missing. Each finding shows its level, name, source and
    message, and jumps to its line.
- **Themes.** The truecolor palettes ship as CSS-variable themes. The default
  follows `prefers-color-scheme`. `ordo.toml` `[theme]` overrides apply on
  top.
- **Keys.** The vim preset's defaults, `ordo.toml` `[binds]`, and the mouse.
- **Command palette (`:`).** Every command `:help` lists.
- **Opening the editor.** `ge` opens the file at the hunk, either through a
  configurable URL (default `vscode://file/{root}/{path}:{line}`) or by having the
  server spawn `$VISUAL`.

## Testing
- **Rust.** Golden JSON for the view model, over a fixture git repo built
  inside the test. The HTTP surface is tested too: token, Host check,
  mutations and SSE.
- **TypeScript.** The pure logic (keymap, list centring, table row counts,
  palette parsing) runs under node's built-in `node:test`.
- **Browser.** A smoke pass over a real branch before a phase closes.

## Phases
1. Engine: formatting only; a root swallows its children.
2. The review layer, separated from drawing.
3. `ordo web`: server, view model, SSE, loading progress, cached revs.
4. Client core: band, list, code, why, themes, keys.
5. Parity: comments, notes, watch, waves, SARIF/coverage, dependency canvas,
   palette, `:config`.
6. `docs/web.md` and the README.
