# Browser reviewer (`ordo web`)

The same review as the terminal's, served to a browser on this machine:

```sh
ordo web [<rev>] [<glob>...]       # rev defaults to HEAD; opens a browser tab
ordo web main...feature --no-open  # print the URL instead
ordo web HEAD --port 4780          # a fixed port (default: any free one)
```

The terminal reviewer's options — `--watch`, `--no-catalog`, the globs — mean
the same here; the page's colours are its own (see `:theme` below). `ordo <rev>` without `web` is
still the terminal reviewer.

The server is the review: Rust reads the diff, orders it, and builds the view
model; the page only lays it out and sends back what you did. Marks, notes and
comments land in the same files the terminal writes, merged rather than
overwritten, so a terminal and a browser open on the same revision do not undo
each other. `Ctrl-C` stops the server.

## Safety

The server listens on `127.0.0.1` only. The URL it prints carries a session
token after `#t=`; every `/api` request must present it, and a request whose
`Host` is not a local name (`localhost`, `127.0.0.1`, `[::1]`, on any port,
so a forwarded port works) is refused, so another site resolving to localhost
cannot read the review. The token lives for one run. The page
moves it out of the address bar into the tab's own storage once loaded, so
open a second tab from the URL the server printed, not from the address bar.

## Layout

```text
┌ ordo  rev  subject           ● verdicts  ● notes  ▬▬▬ 34 of 608 read  Keys ┐
│ Reading order   │ Hunk 37 of 608   path:line           │ Why it is here    │
│ ○ file.py       │ Edits the comment in _back_rz        │ Needs             │
│ ●  ╮            │ Grouped by …                         │ Needed by         │
│ ◉ ─┤            │ ┌ the file, opened at the hunk ────┐ │ If you reject it  │
│ ○  │            │ │                                  │ │ Findings          │
│ ○ ─╯            │ └──────────────────────────────────┘ │ Test coverage     │
│                 │ [Mark as read x]  Next …             │ History           │
└─────────────────┴──────────────────────────────────────┴───────────────────┘
```
The page is set in Geist Mono, compiled into the binary,
so it reads the same offline. It follows the system's light or dark setting:
blue line on a white sheet by day, white line on a blue sheet by night.

The **reading order** on the left is every hunk on one thread, grouped by file:
a hollow square is unread, a filled one read, a red or amber outline a finding. The
selected hunk stays centred unless the thread runs out above or below it, and
its dependencies are drawn as arcs off the thread — solid to what it needs,
dashed to what needs it.

The **reading column** in the middle is one hunk: where it is, what it does in
a sentence with its names set as code, what it is grouped by, and the file
opened at the hunk's first row with three rows above. Below the code, *Mark as
read* (`x`) marks it and moves on, and *Next* names the hunk that follows.

The **margin** on the right says why the hunk is here: what it needs, what
needs it, and what rejecting it would strand — three of each, every one once
the column is maximised (press its number, `3`, again, or the button in its
corner) — then its findings (with none, which rules were checked), test
coverage, and how often the file and these lines changed before.

The header carries the review's state:

| Mark | Meaning |
| --- | --- |
| new | new since you last looked |
| changed | changed since you last looked |
| moved | unchanged, but it reads in a different place now — its dependencies moved |

The *n changed since your last look* button narrows the reading order to those
hunks; a first review compares against nothing, so it shows no count. The
header also says when `--watch` is on and when the working tree has moved on
since the review was read. A review with nothing in it offers to read again
rather than quitting.

A thin strip down the right of the code maps the whole file: the hunk being
read in full ink, other hunks fainter, and beside them verdicts, warnings, line
comments and search hits. The outlined box is the part in view; click or
drag on the strip to go elsewhere.

Columns resize by dragging the rules between them. Below 1100px wide they
stack, the reading column first. `gD` opens the dependency canvas as an
overlay: the hunk, what it needs and what needs it, one step each way.

`gD` again, or `:map`, draws the hunk's whole chain: everything it stands on
to its left, and everything rejecting it would strand to its right, each
hunk right of what it needs. Pointing at a hunk shades what rejecting *that*
one strands and outlines what it needs; a dashed edge is a hunk marked
reviewed before what it needs. A click or `Enter` reads the hunk, and `gD`
on another hunk redraws the chain around it.

## Keys and commands

The keymap is the terminal's — the `vim` preset, with `tui.toml`'s `[binds]`
on top; `ordo help tui` has the table. The mouse selects hunks and code lines.
The column motions (`h` `l` `w` `b` `e` `0` `$`), or a click on a name, put a
caret in the code line, and `K` looks up the name under it. Resting the
pointer on a name shows the same lookup without its history, and lists the
other hunks in the review that declare the name or change its body; a click
on one reads it. Two kinds of
terminal key do nothing here:

- the `Ctrl-w` chords, which a browser keeps for closing the tab — `1` `2` `3`
  focus a pane and, pressed again, maximise it;
- horizontal scrolling and the churn popup — the browser scrolls the line, and
  the churn numbers are already in the band.
`:` opens the command palette with every command the terminal's bar takes,
except `:quickfix` (the list is already here); `:e <rev>` moves the tab to
another revision, and `:yank` copies the hand-back prompt to the clipboard.
`:theme <name>` lays one of the terminal's colour themes, with `tui.toml`'s
`[theme]` colours applied, over the page; `:theme auto` goes back to the page's
own. `:size` makes the text and rows bigger or smaller (`small`, `normal`,
`large`, `larger`, or a factor such as `1.2`). Both stick in this browser.

`:summary`, or the *n of m read* button in the header, puts the review on one
page in place of the columns: how much is read, everything you wrote (notes
and line comments, file by file in reading order), the verdicts and warnings
ordo raised as a grid of cards, and a table of files with their progress. Each
location reads that hunk; a file name reads its first unread hunk. *Copy for
the author* is `:yank` and *Copy with ordo's findings* is `:yank all`. `q` or
the same button goes back.

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
