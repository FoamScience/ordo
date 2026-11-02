# Browser reviewer (`ordo web`)

The same review as the terminal's, served to a browser on this machine:

```sh
ordo web [<rev>] [<glob>...]       # rev defaults to HEAD; opens a browser tab
ordo web main...feature --no-open  # print the URL instead
ordo web HEAD --port 4780          # a fixed port (default: any free one)
```

Every option the terminal reviewer takes — `--watch`, `--no-catalog`,
`--theme`, the globs — means the same here. `ordo <rev>` without `web` is
still the terminal reviewer.

The server is the review: Rust reads the diff, orders it, and builds the view
model; the page only lays it out and sends back what you did. Marks, notes and
comments land in the same files the terminal writes, merged rather than
overwritten, so a terminal and a browser open on the same revision do not undo
each other. `Ctrl-C` stops the server.

## Safety

The server listens on `127.0.0.1` only. The URL it prints carries a session
token after `#t=`; every `/api` request must present it, and a request whose
`Host` is not this server's own address is refused, so another site resolving
to localhost cannot read the review. The token lives for one run. The page
moves it out of the address bar into the tab's own storage once loaded, so
open a second tab from the URL the server printed, not from the address bar.

## Layout

```text
┌ band: rev · subject · coverage · ✦◐⇅ since · stale · findings ───────────┐
│ list              │ code                          │ why                   │
│ (reading order)   │ (the hunk, from its first row)│ (what, grouped by,    │
│                   │                               │  needs, cascade, …)   │
└───────────────────┴───────────────────────────────┴───────────────────────┘
```

The **band** holds what used to scroll away in the why pane:

| Glyph | Meaning |
| --- | --- |
| ✦ | new since you last looked |
| ◐ | changed since you last looked |
| ⇅ | unchanged, but it reads in a different place now — its dependencies moved |

The `◐ n since last look` button filters the list to those hunks; a first
review compares against nothing, so it shows no count. The band also shows the
selected hunk's churn (fetched in the background) and, when `--watch` is on,
that it is watching. A review with nothing in it offers to read again rather
than quitting.

The **list** keeps the selected hunk centred unless the list runs out above or
below it. The **code** pane opens at the hunk's first row with three rows of
context above it. The **why** pane lists `what` the hunk does, what it is
`grouped by`, and its dependency tables — what it needs, what needs it, and
what rejecting it would strand — each capped at three rows. Maximise the pane
(press its number, `3`, again, or the button in its header) and they show
every row. Findings follow in full; with none, the pane says which rules were
checked.

Panes resize by dragging the separators. Below 1100px wide they stack. `gD`
opens the dependency canvas as an overlay.

## Keys and commands

The keymap is the terminal's — the `vim` preset, with `tui.toml`'s `[binds]`
on top; `ordo help tui` has the table. The mouse selects hunks and code lines.
The column motions (`h` `l` `w` `b` `e` `0` `$`), or a click on a name, put a
caret in the code line, and `K` looks up the name under it. Two kinds of
terminal key do nothing here:

- the `Ctrl-w` chords, which a browser keeps for closing the tab — `1` `2` `3`
  focus a pane and, pressed again, maximise it;
- horizontal scrolling and the churn popup — the browser scrolls the line, and
  the churn numbers are already in the band.
`:` opens the command palette with every command the terminal's bar takes,
except `:quickfix` (the list is already here); `:e <rev>` moves the tab to
another revision, and `:yank` copies the hand-back prompt to the clipboard.
`:theme <name>` switches the colour theme for this tab.

Themes are the terminal's, with `tui.toml`'s `[theme]` colours applied. With no
theme chosen the page follows the browser's light or dark preference.

`ge` opens the hunk at the cursor line in an editor, chosen by
`ORDO_WEB_EDITOR`:

| Value | Opens |
| --- | --- |
| unset | `vscode://file/{root}/{path}:{line}` in the browser |
| a URL template | that URL, with `{root}`, `{path}` and `{line}` filled in |
| `server` | `$VISUAL` (or `$EDITOR`) `+line path`, started by the server |

## Developing the client

The client is TypeScript under `web/src`, bundled by esbuild into `web/dist`,
which is committed and compiled into the binary — a `cargo build` needs no
Node. A test fails when `web/dist/source.hash` no longer matches `web/src`.

```sh
cd web && npm install
npm run build    # web/src → web/dist, and the hash
npm run check    # tsc --noEmit
npm test         # node --test test/*.test.ts
ORDO_WEB_DIST=$PWD/web/dist ordo web   # serve assets from disk, no rebuild of ordo
```

`cargo test --test web` drives a real server over a socket; its view-model
snapshot is `tests/web/review.json` (`UPDATE_GOLDEN=1` to regenerate).
