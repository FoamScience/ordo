# Reviewer (`ordo`)

`ordo` reads a change, orders it, and serves the review to a browser on this
machine:

```sh
ordo [<rev>] [<glob>...]           # rev defaults to HEAD; opens a browser tab
ordo main...feature --no-open      # print the URL instead
ordo HEAD --port 4780              # a fixed port (default: any free one)
ordo help [<topic>]                # this page, and the others, offline
```

`ordo web ...` is the same command. `ordo help` lists the topics; `ordo help
<topic>` prints one, paged through `$PAGER` when there is a terminal to page
for. The pages are compiled into the binary, so they can never describe a
different version than the one running.

The server is the review: Rust reads the diff (it owns git, and shells out for
a commit's blobs), calls `ordo::run`, and builds the view model; the page only
lays it out and sends back what you did. The engine never learns what git is.
Marks, notes and comments land in files under the cache directory, merged
rather than overwritten, so two tabs or two servers open on the same revision
do not undo each other. `Ctrl-C` stops the server.

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

Changed lines are refined the way Neovim's `DiffText` refines `DiffChange`: a
removed line is paired with the added line it became, and only the differing
part carries the strong tint —

```diff
- fn content_matches(info: &AgentInfo, node: &AgentNode) -> bool {
+ fn content_matches(info: &AgentInfo, node: &AgentNode, label: Option<&str>) -> bool {
                                                       └── only this is highlighted
```

The unit of comparison is the tree-sitter **leaf node**, never a character or a
whitespace-split word, so `AgentNode` → `AgentNodeRef` reads as one identifier
replaced rather than a three-character suffix appended. A comment, a string or
prose is the exception: its leaf is a run of words, so it is compared word by
word and a bumped year marks the year. Lines with too little in
common are left unrefined and render whole, so a rewrite is never dressed up as
a small edit. The algorithm is `ordo::refine`, a public library module — a
leaf-level LCS, deliberately not a full tree alignment (difftastic's
Dijkstra-over-graphs, or `syndiff`): those buy accuracy on moved and
restructured code at a cost this does not need to pay to say "an argument was
added".

Text the pairing leaves unmatched on both sides that is the same text — two
arguments swapped, a statement moved a few lines down — is marked **moved**,
with a dashed outline, rather than as one thing deleted and another added. Text
that only re-wraps (a call split over three lines) keeps its place among what
did match and is not called moved.

`corpus/refine_gold.jsonl` holds hunks whose changed and moved text a person
marked; `tests/refine_gold.rs` scores the refiner against them char by char
(opt-in on `$ORDO_CORPUS`) and fails when any one of them gets worse.

The margin carries the file's recent churn — how many commits touched it in
the last six months — counted once at load, because one `git rev-list` per file
is ~9ms. On a change touching more than a hundred files that pass is skipped
rather than paid for: nobody triages a mass rename by churn. Reading a hunk
refines it to *those lines*: how often they changed, and who touched them
last. That one waits until the hunk is read because `git log -L` follows a
line range through the whole history, which measured ~0.2s on a 13k-line file,
so a few hundred hunks would cost most of a minute up front. The answer is
cached per hunk.

Neither changes the reading order. Order is the def→use graph's to decide;
churn says what deserves attention, which is a different question.

## Keys and commands

The keymap is the `vim` preset (`--keys vscode` for the other one), with
`ordo.toml`'s `[binds]` on top. The mouse selects hunks and code lines.
The column motions (`h` `l` `w` `b` `e` `0` `$`), or a click on a name, put a
caret in the code line, and `K` looks up the name under it. Resting the
pointer on a name shows the same lookup without its history, and lists the
other hunks in the review that declare the name or change its body; a click
on one reads it. The `Ctrl-w` chords in the table do nothing here: a browser
keeps `Ctrl-w` for closing the tab. `1` `2` `3` focus a pane and, pressed
again, maximise it.

<!-- ordo:begin keys -->
| | `vim` (default) | `vscode` |
| --- | --- | --- |
| **general** | | |
| quit (or dismiss an open popup or search) | `q`, `Esc` | `C-q`, `Esc` |
| open the command bar (`:help` lists every command) | `:` | `C-P` |
| open the command bar pre-filled with `goto ` |  | `C-p` |
| **navigation** | | |
| next item / move cursor down | `j`, `Down` | `Down` |
| previous item / move cursor up | `k`, `Up` | `Up` |
| jump to the first item / top | `gg` |  |
| jump to the last item / bottom | `G` |  |
| page down | `Space`, `f`, `C-f`, `PageDown` | `PageDown` |
| page up | `C-b`, `PageUp` | `PageUp` |
| half-page down | `C-d` |  |
| half-page up | `C-u` |  |
| move the code cursor left | `h` | `Left` |
| move the code cursor right | `l` | `Right` |
| move the code cursor to the next word | `w` | `C-Right` |
| move the code cursor to the previous word | `b` | `C-Left` |
| move the code cursor to the end of the word | `e` |  |
| move to line start / pane top | `0` | `Home` |
| move to line end / pane bottom | `$` | `End` |
| jump to the previous blank line | `{` |  |
| jump to the next blank line | `}` |  |
| jump to the previous use of a name added here | `[` |  |
| jump to the next use of a name added here | `]` |  |
| why pane: jump to the current dep line's target hunk | `gd`, `Enter` | `C-Enter` |
| jump back to the position before the last dep jump | `C-o` | `A-Left` |
| show only the next wave (after the last, all of them again) | `gw` | `A-PageDown` |
| show only the previous wave | `gW` | `A-PageUp` |
| **panes** | | |
| focus the next pane | `C-w C-w`, `C-w w` | `F6` |
| focus the previous pane | `C-w W`, `C-w p` | `S-F6` |
| open the dependency canvas: what this hunk needs, and what needs it | `gD` |  |
| focus the dependency canvas, when open | `4` |  |
| focus the reading-order pane (again to zoom) | `1`, `C-w h`, `C-w Left` | `C-1` |
| focus the code pane (again to zoom) | `2`, `C-w l`, `C-w k`, `C-w Right`, `C-w Up` | `C-2` |
| focus the why pane | `3`, `C-w j`, `C-w Down` | `C-3` |
| fill the frame with the focused pane | `C-w z`, `C-w _` |  |
| **search** | | |
| open the text-search prompt | `/` | `C-f` |
| find a hunk anywhere in the review by the words in it | `s` |  |
| next occurrence of the symbol under the cursor | `*` | `C-F12` |
| previous occurrence of the symbol under the cursor | `#` | `C-S-F12` |
| cycle to the next match | `n` | `F3` |
| cycle to the previous match | `N` | `S-F3` |
| **review** | | |
| toggle reviewed on the selected hunk | `x` | `Space`, `Enter` |
| jump to the next hunk with a finding, the most severe first | `]f` | `F8` |
| jump to the previous hunk with a finding, the most severe first | `[f` | `S-F8` |
| select lines from the code cursor, for :comment | `v` |  |
| comment on the code-pane line or selection | `c` |  |
| jump to the next line comment | `gc` |  |
| jump to the previous line comment | `gC` |  |
| read the change again, keeping your place | `r` | `F5` |
| fold/unfold the selected hunk's group | `za` | `C-k C-l` |
| unfold the selected hunk's group | `zo` |  |
| fold the selected hunk's group | `zc` |  |
| unfold every group | `zR` | `C-k C-j` |
| fold every group | `zM` | `C-k C-0` |
| **editor** | | |
| code pane: show the symbol under the cursor and its history · why pane: preview the current dep line's target | `K` | `F12` |
| open the selected hunk's file in $VISUAL/$EDITOR | `ge` | `C-o` |
| **help** | | |
| show this keybinding help | `?` | `F1` |
<!-- ordo:end keys -->

`s` finds a hunk anywhere in the review by the words in it: its path, the
definition it sits in, the names it defines and the lines it changed. As you
type, the palette lists the best twenty by BM25 over identifier pieces
(`getBladeWire` is `get`, `blade`, `wire`), each as `path:line` and its name;
Up and Down pick one, Enter reads it, and `C-o` comes back.

`:` opens the command palette, with completion over command names and each
command's own arguments; `:e <rev>` moves the tab to another revision, and
`:yank` copies the hand-back prompt to the clipboard:

<!-- ordo:begin commands -->
| command | does |
| --- | --- |
| `:only-comments` | toggle showing only comment/docstring hunks |
| `:all` | toggle showing generated/formatting-noise hunks |
| `:rules` | where the active rules came from, and what was replaced or disabled |
| `:config` | every setting, generated from the tables the program reads |
| `:filter <glob>` | narrow the review to paths matching &lt;glob&gt;; no argument clears it |
| `:strategy <name>` | re-order the review (comprehension, defs-first, file) |
| `:group` | toggle grouping the reading-order list: a group's symbol on its first row only, and folds |
| `:rule` | draft a rule matching the selected hunk's shape |
| `:delta` | what changed since this review was last opened |
| `:note [text]` | anchor a note to the selected hunk's symbol; no text clears it |
| `:comment [text]` | comment on the code-pane line, or the `v` selection; no text deletes what is there |
| `:comments` | list every line comment in this review |
| `:watch [on|off|auto]` | follow the working tree: mark the review stale (on), or reload by itself (auto) |
| `:only-wave <N|last|all>` | show only the hunks one wave changed; dep lines still reach the others |
| `:wave [message]` | record the working tree as the next wave; `ordo wave/2..wave/3` reviews one |
| `:yank [all]` | copy the review as an agent prompt: your notes; `all` adds ordo's own notes and findings |
| `:send [all]` | pipe the review prompt to the command in $ORDO_SEND (the herdr plugin sets it) |
| `:mode [ledger|hunks]` | list by symbol (default) or by hunk; no argument toggles |
| `:goto <path>` | select the first hunk of &lt;path&gt; |
| `:e <rev>` | review a different revision, without restarting |
| `:audit` | account for every hunk and file not on screen, and why |
| `:help` | list these commands |
<!-- ordo:end commands -->

`:theme <name>` lays one of ordo's colour themes (see [Themes](#themes)), with
`ordo.toml`'s `[theme]` colours applied, over the page; `:theme auto` goes
back to the page's own. `:size` makes the text and rows bigger or smaller
(`small`, `normal`, `large`, `larger`, or a factor such as `1.2`). Both stick
in this browser.

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

Reviewed hunks persist to
`${XDG_CACHE_HOME:-$HOME/.cache}/ordo/reviewed/<repo>.json`, keyed by
revision + hunk identity + hunk content — a mark drops itself the moment the
hunk's content changes underneath it. Marks older than 90 days are pruned
automatically. The file records only opaque hashes, never a path, symbol name
or source text.

## `:config`

`:config` opens every setting ordo has, grouped by the file it is written to.
The list is **generated** from the tables the program already reads — the
keymap, `theme_names`, the theme roles, the action names, the bundled rulesets
and the construct catalog's own files — so a new theme role, key action or
catalog rule appears here because it was added there, not because a second list
was kept in step.

Catalog rules are grouped by the file they are written in, and a section
carries its rules: turning off `c-cpp` turns off its fourteen constructs, and
turning one back on brings the section back with it. Changing a rule re-runs
the review, so the list under the dialog is the answer to the setting you just
changed. The `levels` section cycles each catalog rule through `default`,
`note`, `warn` and `verdict` (see [rules.md](rules.md#raising-or-lowering-a-level)).

A colour or a key binding is free text. An empty colour follows its theme
rather than being unset.

Writing saves only what you altered, into the file each section names —
`ordo.toml` for the preset, theme and keys, `rules.toml` for the catalog and
rulesets. Existing lines are replaced where they sit, so the commented-out
defaults that document the file, and any note you left in it, survive. If a
file cannot be read or written the dialog says so and keeps the change pending
rather than reporting a save that did not happen.

## Analyzer findings and coverage

`--sarif <file>` reads SARIF 2.1.0 — what semgrep, CodeQL, ruff, eslint,
shellcheck and `clippy --message-format` emit — and attaches each finding to
the hunk whose lines contain it, so findings arrive in the reading order rather
than as a separate list. A finding on a line the change did not touch is
counted in `:audit`, not shown. A result whose `kind` is `review` is a review
comment rather than an analyzer's verdict (`scripts/review-bot.py --sarif`
writes these): it is listed under its source `review`, and tints its row by
its level like any other finding.

`--coverage <file>` reads an lcov tracefile (`cargo llvm-cov --lcov`,
`coverage.py lcov`, …) and reports, per hunk, how many changed executable lines
never ran. Only `DA:` records count, so a blank line, comment or declaration is
never held against a hunk. This is the fact behind the engine's `code changed
but no test touched` note, which otherwise guesses from file names.

Both are repeatable.

<details>
<summary><b>Revision syntax, path filters, GitButler support</b></summary>

`<rev>` is any git commit-ish, a commit range (`main..branch`, or
`main...branch` to diff from the merge base — an omitted side means `HEAD`), or
`zz` for the uncommitted area. `<base>..zz` (or `<base>...zz` for the merge
base of `<base>` and `HEAD`) reviews everything done on a branch including
what's not yet committed. `zz` on the left (`zz..main`) is meaningless and
rejected.

On a **GitButler**-managed repo `<rev>` also takes the CLI IDs `but status`
prints — the workspace is read once from `but --json status`, and the repo is
otherwise driven by plain git. A repo counts as managed once GitButler has set
it up (its `.git/gitbutler` exists); ordo never runs `but` anywhere else, since
that would set it up:

| `<rev>` | reviews |
| --- | --- |
| a branch ID (`at`) or name (`feat/multi-session`), in a stack or a linked worktree | that branch's own commits, as a range |
| a commit ID (`lzm`), or a change-ID / commit-ID prefix | that commit |
| `@` (or `zz`) | the uncommitted area, including changes assigned to a stack; inside a linked worktree, that worktree's own |

`@` follows GitButler there, `main...@` included; on any other repo it keeps
git's meaning, `HEAD`.

A branch label wins over git's reading of the same name, where a branch is
only its tip commit — reviewing a branch means reviewing its commits.

Either end of a range also takes these IDs: `int..at` or `int...at` runs from
the git branch `int` to the tip of GitButler branch `at`, and a commit ID
stands for that commit. There a branch is its tip, as in git, and git's
reading of a name comes first.

Generated and lock files (`Cargo.lock`, `package-lock.json`, `vendor/`,
`node_modules/`, `.min.js`, …) are dropped before their blobs are read, and so
is anything the repo's own `.gitattributes` declares — `linguist-generated` or
an explicit `-diff`. `--all` keeps everything (the engine still flags known
paths `noise`, so they render dimmed).

```sh
ordo main...feature 'src/*' '*.rs'          # the branch, Rust sources only
ordo zz --all                               # everything uncommitted, lock files included
ordo HEAD 'src/*' '!src/generated/*'        # src/, minus a generated subtree
ordo HEAD '!tests/*'                        # everything except tests/
```

A glob prefixed `!` is negative and excludes a path that matches it; with only
negative globs given, everything except those is kept. `\!literal` escapes a
leading bang. Matching is order-independent, deliberately unlike `.gitignore`.

</details>

## Themes

The page has its own look. `:theme <name>` (or `theme` in `ordo.toml`) lays
one of these over it; `dark` and `light` only set the tints and
leave the text colours to the page, and every other theme names every colour,
so the reviewer matches your editor.

<!-- ordo:begin themes -->
`dark`, `light`, `catppuccin-mocha`, `catppuccin-macchiato`,
`catppuccin-frappe`, `catppuccin-latte`, `tokyonight-night`,
`tokyonight-storm`, `tokyonight-moon`, `tokyonight-day`, `gruvbox-dark`,
`gruvbox-light`, `nord`, `dracula`, `solarized-dark`, `solarized-light`
<!-- ordo:end themes -->

Every role is overridable, on top of any theme. An empty value means the role
follows its palette, which is what `:config` writes back when an override is
cleared:

```toml
[theme]
name = "catppuccin-mocha"
border-focus = "#f5c2e7"    # the focused pane's border
syntax-keyword = "#f38ba8"
add-bg = "#1e3a24"          # the quiet tint on an added line
```

Every role a `[theme]` table may set — each one the name of a field the
program actually reads back, so `--init-config` prints these same names:

<!-- ordo:begin theme-roles -->
`fg`, `dim`, `border`, `border-focus`, `accent`, `reviewed`, `warn`,
`add-bg`, `del-bg`, `add-strong-bg`, `del-strong-bg`, `select-bg`,
`match-bg`, `match-current-bg`, `band-bg`, `verdict-bg`,
`verdict-strong-bg`, `warn-bg`, `warn-strong-bg`, `note-bg`,
`note-strong-bg`, `reviewed-bg`, `syntax-comment`, `syntax-keyword`,
`syntax-string`, `syntax-number`, `syntax-function`, `syntax-type`,
`syntax-property`, `syntax-operator`, `syntax-variable`, `syntax-builtin`,
`syntax-parameter`, `syntax-attribute`
<!-- ordo:end theme-roles -->

A theme colours twelve *syntax roles* rather than the twenty-six tree-sitter
capture names mapped onto them, so a new grammar's captures never mean touching
every theme.

## `ordo.toml`

`ordo --init-config` writes a starting config to
`${XDG_CONFIG_HOME:-~/.config}/ordo/ordo.toml` (`--force` to overwrite): every
binding and every colour of the current preset and theme, at its real value,
commented out. It is generated from the same tables the program reads, so it
can't drift from what ordo accepts — a test uncomments the whole file and
checks it parses cleanly and changes nothing.

Keys are configurable in the same file, on top of whichever preset is in use:

```toml
preset = "vim"            # the preset to start from (--keys still wins)
docs_last = true          # a docs-only hunk sorts after the code it describes

[binds]
"C-n" = "next"            # add or replace a binding
"g d" = "jump-to-edge"    # a chord: prefix, space, key
"x" = "none"              # remove a binding
```

Action names are the ones `:config` lists under keys. A line that names a key
or an action that doesn't exist is reported with its line number and skipped,
so a typo costs one binding rather than the session.

## Developing the client

The client is TypeScript under `web/src`, bundled by esbuild into `web/dist`,
which is committed and compiled into the binary — a `cargo build` needs no
Node. A test fails when `web/dist/source.hash` no longer matches `web/src`.

```sh
cd web && npm install
npm run build    # web/src → web/dist, and the hash
npm run check    # tsc --noEmit
npm test         # node --test test/*.test.ts
ORDO_WEB_DIST=$PWD/web/dist ordo   # serve assets from disk, no rebuild of ordo
```

`cargo test --test web` drives a real server over a socket; its view-model
snapshot is `tests/web/review.json` (`UPDATE_GOLDEN=1` to regenerate).
